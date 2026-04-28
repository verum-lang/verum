//! VBC instruction set definitions.
//!
//! VBC uses a register-based instruction set with variable-length encoding.
//! Each function has a fixed number of registers allocated at compile time.
//!
//! # Opcode Categories (v2)
//!
//! | Range | Category | Description |
//! |-------|----------|-------------|
//! | 0x00-0x0F | Data Movement | MOV, LOAD_*, CVT_* |
//! | 0x10-0x1F | Integer Arithmetic | ADD_I, SUB_I, CVT_TO_I |
//! | 0x20-0x2F | Float Arithmetic | ADD_F, SUB_F, CVT_TO_F |
//! | 0x30-0x3F | Bitwise + Generic Arith | BAND, BOR, ADD_G |
//! | 0x40-0x4F | Comparison | EQ_I, LT_F, CMP_G |
//! | 0x50-0x5F | Control Flow | JMP, RET, CALL, CALL_R |
//! | 0x60-0x6F | Memory + Collections | NEW, GET_F, NEW_LIST, CLONE |
//! | 0x70-0x7F | CBGR | REF, DEREF, CHK, CBGR_EXTENDED |
//! | 0x80-0x8F | Generic + Variant | CALL_G, MAKE_VARIANT, NEW_CLOSURE |
//! | 0x90-0x9F | Pattern + Logic | IS_VAR, AND, OR, XOR, NOT |
//! | 0xA0-0xAF | Async + Nursery | SPAWN, AWAIT, NURSERY_* |
//! | 0xB0-0xBF | Context + Meta | CTX_GET, META_*, FFI_EXT, ARITH_EXT |
//! | 0xC0-0xCF | Iterator + Generator + String | ITER_*, GEN_*, TO_STRING, CONCAT |
//! | 0xD0-0xDF | Exception + Debug | THROW, TRY_*, SPEC, ASSERT |
//! | 0xE0-0xEF | System (V-LLSI) + Autodiff | SYSCALL, MMAP, GRAD_* |
//! | 0xF0-0xFF | Tensor + GPU | TENSOR_*, GPU_*, ML_EXTENDED |

use serde::{Deserialize, Serialize};

use crate::types::TypeRef;

/// Register reference.
///
/// Registers are encoded as:
/// - r0-r127: Single byte (0x00-0x7F)
/// - r128-r16383: Two bytes (0x80 | high7, low8)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Reg(pub u16);

impl Reg {
    /// Maximum register index.
    pub const MAX: u16 = 16383;

    /// Creates a new register reference.
    pub fn new(index: u16) -> Self {
        debug_assert!(index <= Self::MAX, "Register index out of bounds");
        Reg(index)
    }

    /// Returns true if this register can be encoded in a single byte.
    pub fn is_short(&self) -> bool {
        self.0 < 128
    }
}

/// Register range for function calls.
///
/// Encodes a contiguous range of registers: `start..start+count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct RegRange {
    /// First register in range.
    pub start: Reg,
    /// Number of registers.
    pub count: u8,
}

impl RegRange {
    /// Creates a new register range.
    pub fn new(start: Reg, count: u8) -> Self {
        Self { start, count }
    }

    /// Returns an iterator over the registers in this range.
    pub fn iter(&self) -> impl Iterator<Item = Reg> {
        let start = self.start.0;
        let count = self.count as u16;
        (0..count).map(move |i| Reg(start + i))
    }
}

/// VBC opcode enumeration.
///
/// All 256 opcodes are defined here, organized by category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Opcode {
    // ========================================================================
    // Data Movement (0x00-0x0F)
    // ========================================================================
    /// Move register: `dst = src`
    Mov = 0x00,
    /// Load constant: `dst = const_pool[id]`
    LoadK = 0x01,
    /// Load immediate integer: `dst = imm`
    LoadI = 0x02,
    /// Load immediate float: `dst = imm`
    LoadF = 0x03,
    /// Load true: `dst = true`
    LoadTrue = 0x04,
    /// Load false: `dst = false`
    LoadFalse = 0x05,
    /// Load unit: `dst = ()`
    LoadUnit = 0x06,
    /// Load type reference: `dst = TypeRef`
    LoadT = 0x07,
    /// Load small immediate (-64..63): `dst = simm`
    LoadSmallI = 0x08,
    /// Load nil/null: `dst = nil`
    LoadNil = 0x09,
    /// No operation (useful for padding/debugging).
    Nop = 0x0A,
    /// Convert Int to Float: `dst = src as Float`
    CvtIF = 0x0B,
    /// Convert Float to Int: `dst = src as Int` with mode (0=trunc, 1=floor, 2=ceil, 3=round)
    CvtFI = 0x0C,
    /// Convert Int to Char: `dst = src as Char`
    CvtIC = 0x0D,
    /// Convert Char to Int: `dst = src as Int`
    CvtCI = 0x0E,
    /// Convert Bool to Int: `dst = src as Int` (false → 0, true → 1)
    CvtBI = 0x0F,

    // ========================================================================
    // Integer Arithmetic (0x10-0x1F)
    // ========================================================================
    /// Integer add: `dst = a + b`
    AddI = 0x10,
    /// Integer subtract: `dst = a - b`
    SubI = 0x11,
    /// Integer multiply: `dst = a * b`
    MulI = 0x12,
    /// Integer divide: `dst = a / b` (traps on zero)
    DivI = 0x13,
    /// Integer modulo: `dst = a % b` (traps on zero)
    ModI = 0x14,
    /// Integer negate: `dst = -src`
    NegI = 0x15,
    /// Absolute value (int): `dst = |src|`
    AbsI = 0x16,
    /// Integer power: `dst = a ** b`
    PowI = 0x17,
    /// Increment: `dst = src + 1`
    Inc = 0x18,
    /// Decrement: `dst = src - 1`
    Dec = 0x19,
    /// Dynamic Convert to Int: runtime type dispatch
    /// Handles: Float→Int (truncate), Bool→Int (0/1), Char→Int (codepoint), Int→Int (identity)
    CvtToI = 0x1A,
    /// Reserved integer arithmetic.
    IntArith1B = 0x1B,
    /// Reserved integer arithmetic.
    IntArith1C = 0x1C,
    /// Reserved integer arithmetic.
    IntArith1D = 0x1D,
    /// Reserved integer arithmetic.
    IntArith1E = 0x1E,
    /// General-purpose extension opcode (#167 Part A).
    ///
    /// Encoded as `[0x1F] [sub_op:u8] [operands...]`.  The sub-op byte
    /// selects the extended-instruction kind, giving us a clean
    /// 256-entry sub-op space carved out of the previously-reserved
    /// `IntArith1F` slot.  Used as the home for new first-class
    /// instructions that don't fit any existing extension namespace
    /// (Math/Tensor/Cbgr/Ffi/etc.) — first occupant is
    /// `MakeVariantTyped` (#146 Phase 3, Extended sub-op `0x01`).
    /// Sub-op `0x00` is reserved as a no-op for forward-compat.
    Extended = 0x1F,

    // ========================================================================
    // Float Arithmetic (0x20-0x2F)
    // ========================================================================
    /// Float add: `dst = a + b`
    AddF = 0x20,
    /// Float subtract: `dst = a - b`
    SubF = 0x21,
    /// Float multiply: `dst = a * b`
    MulF = 0x22,
    /// Float divide: `dst = a / b`
    DivF = 0x23,
    /// Float modulo: `dst = a % b`
    ModF = 0x24,
    /// Float negate: `dst = -src`
    NegF = 0x25,
    /// Absolute value (float): `dst = |src|`
    AbsF = 0x26,
    /// Float power: `dst = a ** b`
    PowF = 0x27,
    /// Dynamic Convert to Float: runtime type dispatch
    /// Handles: Int→Float, Float→Float (identity)
    CvtToF = 0x28,
    /// Math Extended operations prefix for transcendental and special functions.
    ///
    /// This opcode is followed by a sub-opcode byte (MathSubOpcode) that specifies
    /// the specific math operation. All operations use native Rust/LLVM implementations
    /// for zero-cost execution in both interpreter and AOT compilation.
    ///
    /// # Sub-opcode Ranges
    ///
    /// - 0x00-0x0F: Trigonometric (sin, cos, tan, asin, acos, atan, atan2)
    /// - 0x10-0x1F: Hyperbolic (sinh, cosh, tanh, asinh, acosh, atanh)
    /// - 0x20-0x2F: Exponential/Logarithmic (exp, exp2, expm1, log, log2, log10, log1p, pow)
    /// - 0x30-0x3F: Root/Power (sqrt, cbrt, hypot)
    /// - 0x40-0x4F: Rounding (floor, ceil, round, trunc)
    /// - 0x50-0x5F: Special (abs, copysign, fma, fmod, remainder, fdim, minnum, maxnum)
    /// - 0x60-0x6F: Classification (is_nan, is_inf, is_finite)
    ///
    /// # Encoding
    ///
    /// ```text
    /// [0x29] [sub_opcode:u8] [dst:reg] [src:reg] [src2:reg]?
    /// ```
    ///
    /// # Performance
    ///
    /// - Interpreter: ~2ns per operation (native Rust method call)
    /// - AOT: 0ns overhead (maps to LLVM intrinsics)
    MathExtended = 0x29,
    /// SIMD Extended operations prefix for vector operations.
    ///
    /// This opcode is followed by a sub-opcode byte (SimdSubOpcode) that specifies
    /// the SIMD operation. Platform-agnostic operations that lower to:
    /// - x86: AVX2/AVX-512 intrinsics
    /// - ARM: NEON intrinsics
    /// - MLIR: vector dialect
    ///
    /// Format: `[0x2A] [sub_opcode:u8] [operands...]`
    SimdExtended = 0x2A,
    /// Char Extended operations prefix for character operations.
    ///
    /// This opcode is followed by a sub-opcode byte (CharSubOpcode) that specifies
    /// the character operation. ASCII operations are inline, Unicode operations
    /// use runtime lookup.
    ///
    /// Format: `[0x2B] [sub_opcode:u8] [dst:reg] [src:reg]`
    CharExtended = 0x2B,
    /// Reserved float arithmetic.
    FloatArith2C = 0x2C,
    /// Reserved float arithmetic.
    FloatArith2D = 0x2D,
    /// Reserved float arithmetic.
    FloatArith2E = 0x2E,
    /// Reserved float arithmetic.
    FloatArith2F = 0x2F,

    // ========================================================================
    // Bitwise + Generic Arithmetic (0x30-0x3F)
    // ========================================================================
    /// Bitwise AND: `dst = a & b`
    Band = 0x30,
    /// Bitwise OR: `dst = a | b`
    Bor = 0x31,
    /// Bitwise XOR: `dst = a ^ b`
    Bxor = 0x32,
    /// Bitwise NOT: `dst = ~src`
    Bnot = 0x33,
    /// Shift left: `dst = a << b`
    Shl = 0x34,
    /// Arithmetic shift right: `dst = a >> b`
    Shr = 0x35,
    /// Logical shift right: `dst = a >>> b`
    Ushr = 0x36,
    /// Reserved bitwise.
    Bitwise37 = 0x37,
    /// Generic add via Add protocol.
    AddG = 0x38,
    /// Generic subtract via Sub protocol.
    SubG = 0x39,
    /// Generic multiply via Mul protocol.
    MulG = 0x3A,
    /// Generic divide via Div protocol.
    DivG = 0x3B,
    /// Reserved generic arithmetic.
    GenArith3C = 0x3C,
    /// Reserved generic arithmetic.
    GenArith3D = 0x3D,
    /// Reserved generic arithmetic.
    GenArith3E = 0x3E,
    /// Reserved generic arithmetic.
    GenArith3F = 0x3F,

    // ========================================================================
    // Comparison (0x40-0x4F)
    // ========================================================================
    /// Integer equal: `dst = (a == b)`
    EqI = 0x40,
    /// Integer not equal: `dst = (a != b)`
    NeI = 0x41,
    /// Integer less than: `dst = (a < b)`
    LtI = 0x42,
    /// Integer less or equal: `dst = (a <= b)`
    LeI = 0x43,
    /// Integer greater than: `dst = (a > b)`
    GtI = 0x44,
    /// Integer greater or equal: `dst = (a >= b)`
    GeI = 0x45,
    /// Float equal: `dst = (a == b)`
    EqF = 0x46,
    /// Float not equal: `dst = (a != b)`
    NeF = 0x47,
    /// Float less than: `dst = (a < b)`
    LtF = 0x48,
    /// Float less or equal: `dst = (a <= b)`
    LeF = 0x49,
    /// Float greater than: `dst = (a > b)`
    GtF = 0x4A,
    /// Float greater or equal: `dst = (a >= b)`
    GeF = 0x4B,
    /// Generic equal via Eq protocol.
    EqG = 0x4C,
    /// Generic compare via Ord protocol -> Ordering.
    CmpG = 0x4D,
    /// Reference equality (pointer compare).
    EqRef = 0x4E,
    /// Extended comparison operations (unsigned integer comparisons).
    ///
    /// Uses sub-opcodes for operations that don't fit in the primary comparison range.
    /// Encoding: `[0x4F] [sub_opcode:u8] [dst:reg] [a:reg] [b:reg]`
    ///
    /// Sub-opcodes:
    /// - 0x00: LtU (unsigned less than)
    /// - 0x01: LeU (unsigned less or equal)
    /// - 0x02: GtU (unsigned greater than)
    /// - 0x03: GeU (unsigned greater or equal)
    CmpExtended = 0x4F,

    // ========================================================================
    // Control Flow (0x50-0x5F)
    // ========================================================================
    /// Unconditional jump.
    Jmp = 0x50,
    /// Jump if true: `if cond { jmp offset }`
    JmpIf = 0x51,
    /// Jump if false: `if !cond { jmp offset }`
    JmpNot = 0x52,
    /// Fused compare and jump: `if a == b { jmp }`
    JmpEq = 0x53,
    /// Fused compare and jump: `if a != b { jmp }`
    JmpNe = 0x54,
    /// Fused compare and jump: `if a < b { jmp }`
    JmpLt = 0x55,
    /// Fused compare and jump: `if a <= b { jmp }`
    JmpLe = 0x56,
    /// Fused compare and jump: `if a > b { jmp }`
    JmpGt = 0x57,
    /// Fused compare and jump: `if a >= b { jmp }`
    JmpGe = 0x58,
    /// Return with value.
    Ret = 0x59,
    /// Return void.
    RetV = 0x5A,
    /// Call function: `dst = fn(args...)`
    Call = 0x5B,
    /// Tail call (reuses stack frame).
    TailCall = 0x5C,
    /// Method call: `dst = receiver.method(args...)`
    CallM = 0x5D,
    /// Call closure.
    CallClosure = 0x5E,
    /// Indirect call (via register).
    CallR = 0x5F,

    // ========================================================================
    // Memory + Collections (0x60-0x6F)
    // ========================================================================
    /// Allocate object: `dst = new Type`
    New = 0x60,
    /// Allocate generic: `dst = new Type<args>`
    NewG = 0x61,
    /// Get field: `dst = obj.field`
    GetF = 0x62,
    /// Set field: `obj.field = val`
    SetF = 0x63,
    /// Get element: `dst = arr[idx]`
    GetE = 0x64,
    /// Set element: `arr[idx] = val`
    SetE = 0x65,
    /// Get length: `dst = arr.len()`
    Len = 0x66,
    /// Allocate array: `dst = [elem; len]`
    NewArray = 0x67,
    /// Allocate list with capacity.
    NewList = 0x68,
    /// Push to list: `list.push(val)`
    ListPush = 0x69,
    /// Pop from list: `dst = list.pop()`
    ListPop = 0x6A,
    /// Allocate map.
    NewMap = 0x6B,
    /// Map get: `dst = map[key]`
    MapGet = 0x6C,
    /// Map set: `map[key] = val`
    MapSet = 0x6D,
    /// Map contains: `dst = key in map`
    MapContains = 0x6E,
    /// Clone value.
    Clone = 0x6F,

    // ========================================================================
    // CBGR Instructions (0x70-0x7F)
    // ========================================================================
    /// Create immutable reference: `dst = &src`
    Ref = 0x70,
    /// Create mutable reference: `dst = &mut src`
    RefMut = 0x71,
    /// Dereference: `dst = *ref`
    Deref = 0x72,
    /// Dereference mutable.
    DerefMut = 0x73,
    /// CBGR validation check.
    ChkRef = 0x74,
    /// Create checked reference (Tier 1).
    RefChecked = 0x75,
    /// Create unsafe reference (Tier 2).
    RefUnsafe = 0x76,
    /// Drop reference.
    DropRef = 0x77,
    /// CBGR extended operations prefix.
    ///
    /// This opcode is followed by a sub-opcode byte (CbgrSubOpcode) that specifies
    /// the actual CBGR operation. This design allows extended CBGR functionality
    /// without consuming main opcode space.
    ///
    /// Format: `[0x78] [sub_opcode:u8] [operands...]`
    CbgrExtended = 0x78,
    /// Text Extended instruction for text parsing and conversion operations.
    ///
    /// This opcode is followed by a sub-opcode byte (TextSubOpcode) that specifies
    /// the actual text operation. This design provides ~2ns zero-cost dispatch
    /// for text operations instead of string-based library calls (~15ns).
    ///
    /// Format: `[0x79] [sub_opcode:u8] [operands...]`
    TextExtended = 0x79,
    /// Reserved CBGR.
    Cbgr7A = 0x7A,
    /// Reserved CBGR.
    Cbgr7B = 0x7B,
    /// Reserved CBGR.
    Cbgr7C = 0x7C,
    /// Reserved CBGR.
    Cbgr7D = 0x7D,
    /// Reserved CBGR.
    Cbgr7E = 0x7E,
    /// Reserved CBGR.
    Cbgr7F = 0x7F,

    // ========================================================================
    // Generic + Variant (0x80-0x8F)
    // ========================================================================
    /// Generic function call.
    CallG = 0x80,
    /// Virtual dispatch.
    CallV = 0x81,
    /// Inline cached call.
    CallC = 0x82,
    /// Size of generic type.
    SizeOfG = 0x83,
    /// Align of generic type.
    AlignOfG = 0x84,
    /// Instantiate generic.
    Instantiate = 0x85,
    /// Create variant with tag.
    MakeVariant = 0x86,
    /// Set variant data field.
    SetVariantData = 0x87,
    /// Get variant data field.
    GetVariantData = 0x88,
    /// Get variant/enum tag.
    GetTag = 0x89,
    /// Create closure: `dst = closure(fn_id, captures...)`
    ///
    /// Creates a closure object with the specified function and captured values.
    /// The closure layout is: [function_id: u32, capture_count: u16, pad: u16, captures: Value[]]
    NewClosure = 0x8A,
    /// Get pointer to variant data field (for ref/ref mut pattern bindings).
    /// Unlike GetVariantData which copies the value, this returns a pointer
    /// to the field location enabling mutation through references.
    GetVariantDataRef = 0x8B,
    /// TypeOf: return runtime type tag of a value.
    TypeOf = 0x8C,
    /// **MakePi** — construct a dependent function value `Π(x: T). U(x)` (T1-H).
    ///
    /// At Tier-0 the dependent function is represented as a closure over the
    /// parameter value plus a type-representing pointer. The opcode takes
    /// (param_value, return_type_id) and packs a PiValue for the interpreter.
    /// Typecheck enforces dependent-typing rules; at runtime the opcode
    /// behaves like an ordinary closure call but preserves the type-level
    /// indexing for reflection (`typeof`, @verify pipelines).
    MakePi = 0x8D,
    /// **MakeSigma** — construct a dependent pair `Σ(x: T). U(x)` (T1-H).
    ///
    /// The witness is the first component (value of type T); the payload
    /// is the second component (of type `U(witness)`). The opcode takes
    /// (witness, payload) and packs a SigmaValue carrying both alongside
    /// the dependent-type descriptor for reflection and pattern elimination.
    MakeSigma = 0x8E,
    /// **MakeWitness** — attach a proof to a refined value (T1-H).
    ///
    /// Used for refinement-type constructs that carry a proof obligation,
    /// e.g. `Int { self > 0 }`: the opcode pairs the value with the
    /// statically-generated proof hash so the interpreter can validate
    /// refinement predicates lazily at gradual-verification boundaries
    /// (T1-F). Erased at Tier-1 (AOT) when the predicate is discharged
    /// by SMT; retained at Tier-0 for runtime assertions.
    MakeWitness = 0x8F,

    // ========================================================================
    // Pattern Matching + Logic (0x90-0x9F)
    // ========================================================================
    /// Check variant: `dst = (val.tag == tag)`
    IsVar = 0x90,
    /// Extract variant payload.
    AsVar = 0x91,
    /// Unpack tuple: `(r0, r1, ...) = tuple`
    Unpack = 0x92,
    /// Pack into tuple: `dst = (r0, r1, ...)`
    Pack = 0x93,
    /// Switch/jump table.
    Switch = 0x94,
    /// Match guard check.
    MatchGuard = 0x95,
    /// Logical AND: `dst = a && b`
    And = 0x96,
    /// Logical OR: `dst = a || b`
    Or = 0x97,
    /// Logical XOR: `dst = a ^^ b`
    Xor = 0x98,
    /// Boolean not: `dst = !src`
    Not = 0x99,
    /// Reserved pattern/logic.
    Pattern9A = 0x9A,
    /// Reserved pattern/logic.
    Pattern9B = 0x9B,
    /// Reserved pattern/logic.
    Pattern9C = 0x9C,
    /// Reserved pattern/logic.
    Pattern9D = 0x9D,
    /// Reserved pattern/logic.
    Pattern9E = 0x9E,
    /// Reserved pattern/logic.
    Pattern9F = 0x9F,

    // ========================================================================
    // Async + Nursery (0xA0-0xAF)
    // ========================================================================
    /// Spawn async task.
    Spawn = 0xA0,
    /// Await task completion.
    Await = 0xA1,
    /// Yield from generator.
    Yield = 0xA2,
    /// Select on multiple futures.
    Select = 0xA3,
    /// Join multiple futures.
    Join = 0xA4,
    /// Check if future is ready.
    FutureReady = 0xA5,
    /// Get future result.
    FutureGet = 0xA6,
    /// Get next from async iterator.
    AsyncNext = 0xA7,
    /// Initialize a new nursery scope for structured concurrency.
    NurseryInit = 0xA8,
    /// Spawn a task into nursery.
    NurserySpawn = 0xA9,
    /// Wait for all nursery tasks to complete.
    NurseryAwait = 0xAA,
    /// Cancel all tasks in nursery.
    NurseryCancel = 0xAB,
    /// Configure nursery options (timeout, max_tasks, error_behavior).
    NurseryConfig = 0xAC,
    /// Get nursery error (if any task failed).
    NurseryError = 0xAD,
    /// Reserved async.
    AsyncAE = 0xAE,
    /// Reserved async.
    AsyncAF = 0xAF,

    // ========================================================================
    // Context + Meta (0xB0-0xBF)
    // ========================================================================
    /// Get context value.
    CtxGet = 0xB0,
    /// Provide context.
    CtxProvide = 0xB1,
    /// End context scope.
    CtxEnd = 0xB2,
    /// Push context handler.
    PushContext = 0xB3,
    /// Pop context handler.
    PopContext = 0xB4,
    /// Attenuate context capabilities.
    Attenuate = 0xB5,
    /// Check if reference has specific capability.
    HasCapability = 0xB6,
    /// Require capability, panic if not present.
    RequireCapability = 0xB7,
    /// Meta evaluation.
    MetaEval = 0xB8,
    /// Quote AST.
    MetaQuote = 0xB9,
    /// Splice into AST.
    MetaSplice = 0xBA,
    /// Type reflection.
    MetaReflect = 0xBB,
    /// FFI Extended operations - foreign function interface.
    ///
    /// Uses sub-opcodes for different FFI operations:
    /// - 0x00: LoadSymbol - Resolve FFI symbol, cache address
    /// - 0x10: CallFfiC - Call with C calling convention
    /// - 0x11: CallFfiStdcall - Call with stdcall (Windows)
    /// - 0x12: CallFfiSysV64 - Call with System V AMD64 ABI
    /// - 0x14: CallFfiVariadic - Variadic call (printf-style)
    /// - 0x15: CallFfiIndirect - Indirect call through fn pointer
    /// - 0x20: MarshalToC - Marshal Verum value to C
    /// - 0x21: MarshalFromC - Marshal C value to Verum
    /// - 0x30: GetErrno - Read errno after FFI call
    ///
    /// Format: `[0xBC] [sub_opcode:u8] [operands...]`
    FfiExtended = 0xBC,
    /// Arithmetic Extended operations - checked, overflowing, and polymorphic arithmetic.
    ///
    /// Uses sub-opcodes for different arithmetic operations:
    /// - 0x00-0x0F: Checked arithmetic (returns Maybe<T>)
    /// - 0x10-0x1F: Overflowing arithmetic (returns (result, overflow_flag))
    /// - 0x20-0x2F: Polymorphic arithmetic (type-dispatched)
    /// - 0x30-0x3F: Saturating arithmetic (future)
    /// - 0x40-0x4F: Wrapping arithmetic (future)
    ///
    /// Format: `[0xBD] [sub_opcode:u8] [operands...]`
    ArithExtended = 0xBD,
    /// Logging Extended operations prefix for structured logging.
    ///
    /// This opcode is followed by a sub-opcode byte (LogSubOpcode) that specifies
    /// the log level. Logging operations are low-frequency and I/O-bound, so
    /// runtime call overhead (~50ns) is acceptable.
    ///
    /// Format: `[0xBE] [sub_opcode:u8] [msg:reg]`
    LogExtended = 0xBE,
    /// Memory extended operations - heap allocation, swap, replace.
    ///
    /// Uses sub-opcodes for different memory operations:
    /// - 0x00: Alloc - allocate heap memory
    /// - 0x01: AllocZeroed - allocate zeroed heap memory
    /// - 0x02: Dealloc - deallocate heap memory
    /// - 0x03: Realloc - reallocate heap memory
    /// - 0x04: Swap - swap two values in place
    /// - 0x05: Replace - replace value and return old
    MemExtended = 0xBF,

    // ========================================================================
    // Iterator + Generator + String + Set (0xC0-0xCF)
    // ========================================================================
    /// Create iterator from iterable.
    IterNew = 0xC0,
    /// Get next element from iterator.
    IterNext = 0xC1,
    /// Create a generator from a generator function.
    GenCreate = 0xC2,
    /// Get next value from a generator (Iterator protocol).
    GenNext = 0xC3,
    /// Check if generator has more values (Iterator protocol).
    GenHasNext = 0xC4,
    /// Convert to string.
    ToString = 0xC5,
    /// Concatenate strings.
    Concat = 0xC6,
    /// Create new set.
    NewSet = 0xC7,
    /// Insert into set.
    SetInsert = 0xC8,
    /// Check set contains.
    SetContains = 0xC9,
    /// Remove from set.
    SetRemove = 0xCA,
    /// Convert Char to string (1-character string).
    CharToStr = 0xCB,
    /// Create new range (for iteration).
    NewRange = 0xCC,
    /// Create a new deque with capacity.
    NewDeque = 0xCD,
    /// Push value to argument stack.
    Push = 0xCE,
    /// Pop value from argument stack.
    Pop = 0xCF,

    // ========================================================================
    // Exception + Debug/Verify (0xD0-0xDF)
    // ========================================================================
    /// Throw exception.
    Throw = 0xD0,
    /// Begin try block.
    TryBegin = 0xD1,
    /// End try block.
    TryEnd = 0xD2,
    /// Get caught exception.
    GetException = 0xD3,
    /// Type specialization hint.
    Spec = 0xD4,
    /// Type guard (deopt if mismatch).
    Guard = 0xD5,
    /// Assert condition.
    Assert = 0xD6,
    /// Panic with message.
    Panic = 0xD7,
    /// Unreachable.
    Unreachable = 0xD8,
    /// Debug print.
    DebugPrint = 0xD9,
    /// Contract precondition.
    Requires = 0xDA,
    /// Contract postcondition.
    Ensures = 0xDB,
    /// Loop invariant.
    Invariant = 0xDC,
    /// Create a new channel with capacity.
    NewChannel = 0xDD,
    /// Cubical type theory extended operations prefix.
    ///
    /// This opcode is followed by a sub-opcode byte (CubicalSubOpcode) that specifies
    /// the actual cubical type theory operation.
    ///
    /// Format: `[0xDE] [sub_opcode:u8] [operands...]`
    CubicalExtended = 0xDE,
    /// Reserved debug.
    DebugDF = 0xDF,

    // ========================================================================
    // System (V-LLSI) + Autodiff (0xE0-0xEF)
    // ========================================================================
    /// Raw Linux syscall: `dst = syscall(num, a1, a2, a3, a4, a5, a6)`
    SyscallLinux = 0xE0,
    /// Memory map region: `dst = mmap(addr, len, prot, flags, fd, offset)`
    Mmap = 0xE1,
    /// Unmap memory region: `result = munmap(addr, len)`
    Munmap = 0xE2,
    /// Atomic load: `dst = atomic_load(ptr, ordering)`
    AtomicLoad = 0xE3,
    /// Atomic store: `atomic_store(ptr, val, ordering)`
    AtomicStore = 0xE4,
    /// Atomic compare-and-swap: `(success, old) = atomic_cas(ptr, expected, new)`
    AtomicCas = 0xE5,
    /// Memory fence: `fence(ordering)`
    AtomicFence = 0xE6,
    /// Submit I/O operation to IOEngine: `token = io_submit(ops)`
    IoSubmit = 0xE7,
    /// Poll IOEngine for completions: `results = io_poll(timeout)`
    IoPoll = 0xE8,
    /// Get thread-local storage: `dst = tls_get(slot)`
    TlsGet = 0xE9,
    /// Set thread-local storage: `tls_set(slot, val)`
    TlsSet = 0xEA,
    /// Begin gradient scope.
    GradBegin = 0xEB,
    /// End gradient scope.
    GradEnd = 0xEC,
    /// Gradient checkpoint.
    GradCheckpoint = 0xED,
    /// Accumulate gradients.
    GradAccumulate = 0xEE,
    /// Stop gradient flow.
    GradStop = 0xEF,

    // ========================================================================
    // Tensor + GPU (0xF0-0xFF)
    // ========================================================================
    /// Create tensor with shape.
    TensorNew = 0xF0,
    /// Binary element-wise op.
    TensorBinop = 0xF1,
    /// Unary element-wise op.
    TensorUnop = 0xF2,
    /// Matrix multiplication.
    TensorMatmul = 0xF3,
    /// Reduction ops (sum, max, mean, etc.).
    TensorReduce = 0xF4,
    /// Reshape tensor.
    TensorReshape = 0xF5,
    /// Transpose tensor.
    TensorTranspose = 0xF6,
    /// Slice tensor.
    TensorSlice = 0xF7,
    /// GPU extended operations prefix.
    ///
    /// This opcode is followed by a sub-opcode byte (GpuSubOpcode) that specifies
    /// the actual GPU operation.
    ///
    /// Format: `[0xF8] [sub_opcode:u8] [operands...]`
    GpuExtended = 0xF8,
    /// Sync GPU stream (fast path).
    GpuSync = 0xF9,
    /// GPU memory copy (fast path).
    GpuMemcpy = 0xFA,
    /// GPU memory allocate (fast path).
    GpuAlloc = 0xFB,
    /// Tensor extended operations prefix.
    ///
    /// This opcode is followed by a sub-opcode byte (TensorSubOpcode) that specifies
    /// additional tensor operations like: TensorFull, TensorArange, TensorLinspace,
    /// TensorRand, TensorClone, TensorIdentity, TensorIndex, TensorConcat, TensorStack,
    /// TensorBroadcast, TensorSqueeze, TensorCmp, TensorWhere, TensorClamp, TensorCast,
    /// TensorMaskedFill, TensorLerp, TensorDot, TensorConv, TensorBatchMatmul, TensorEinsum,
    /// TensorOuter, TensorTriSolve, TensorCholesky, TensorArgmax, TensorTopk, TensorCumulative,
    /// TensorSoftmax, TensorLayerNorm, TensorBatchNorm, TensorRmsNorm, TensorFlashAttention,
    /// TensorFft, TensorScatter, TensorFromSlice.
    ///
    /// Format: `[0xFC] [sub_opcode:u8] [operands...]`
    TensorExtended = 0xFC,
    /// ML Extended operations prefix for ML/AI operations.
    ///
    /// This opcode is followed by a sub-opcode byte (MlSubOpcode) that specifies
    /// ML-specific operations:
    /// - 0x00-0x0F: Tokenizer ops
    /// - 0x10-0x1F: Sampling ops
    /// - 0x20-0x2F: Inference ops
    /// - 0x30-0x3F: Distributed ops
    ///
    /// Format: `[0xFD] [sub_opcode:u8] [operands...]`
    MlExtended = 0xFD,
    /// Create tensor filled with a value.
    ///
    /// Format: `dst:reg, dtype:u8, rank:u8, dims..., value:f64`
    TensorFull = 0xFE,
    /// Create tensor from data slice.
    ///
    /// Format: `dst:reg, data_reg:reg, shape_reg:reg, dtype:u8`
    TensorFromSlice = 0xFF,
}

// ============================================================================
// Cubical Type Theory Extended Sub-Opcodes
// ============================================================================

/// Cubical type theory sub-opcodes for use with `CubicalExtended` (0xDE) prefix.
///
/// These opcodes implement the runtime semantics of cubical type theory
/// primitives: Path types, transport, homogeneous composition (hcomp),
/// and computational univalence.
///
/// At runtime, Path values are represented as closures (functions I → A),
/// and cubical operations manipulate these closures according to the
/// CCHM reduction rules implemented in `verum_types::cubical`.
///
/// # Encoding
///
/// ```text
/// [0xDE] [sub_opcode:u8] [operands...]
/// ```
///
/// # Sub-opcode Ranges
///
/// - `0x00-0x0F`: Path construction (refl, lambda, app, sym, trans, ap)
/// - `0x10-0x1F`: Transport and homogeneous composition
/// - `0x20-0x2F`: Interval operations (i0, i1, meet, join, rev)
/// - `0x30-0x3F`: Univalence (ua, ua_inv, equiv forward/backward)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CubicalSubOpcode {
    // ========================================================================
    // Path Construction (0x00-0x0F)
    // ========================================================================
    /// Create a reflexivity path: refl(x) = λi. x
    ///
    /// Format: `dst:reg, value:reg`
    PathRefl = 0x00,
    /// Create a path lambda: λ(i:I). body
    ///
    /// Format: `dst:reg, body_func:reg`
    PathLambda = 0x01,
    /// Apply a path at an interval point: p @ r
    ///
    /// Format: `dst:reg, path:reg, point:reg`
    PathApp = 0x02,
    /// Path symmetry: sym(p) = λi. p @ (1-i)
    ///
    /// Format: `dst:reg, path:reg`
    PathSym = 0x03,
    /// Path transitivity: trans(p, q) = composition
    ///
    /// Format: `dst:reg, path_p:reg, path_q:reg`
    PathTrans = 0x04,
    /// Action on paths: ap(f, p) = λi. f(p @ i)
    ///
    /// Format: `dst:reg, func:reg, path:reg`
    PathAp = 0x05,

    // ========================================================================
    // Transport and Composition (0x10-0x1F)
    // ========================================================================
    /// Transport along a type path: transport(p, x)
    ///
    /// Key reductions: transport(refl, x) → x; transport(ua(e), x) → e.forward(x)
    ///
    /// Format: `dst:reg, type_path:reg, value:reg`
    Transport = 0x10,
    /// Homogeneous composition: hcomp(φ, walls, base)
    ///
    /// Key reduction: hcomp(φ, const, base) → base (trivial)
    ///
    /// Format: `dst:reg, face:reg, walls:reg, base:reg`
    Hcomp = 0x11,

    // ========================================================================
    // Interval Operations (0x20-0x2F)
    // ========================================================================
    /// Load interval endpoint i0
    ///
    /// Format: `dst:reg`
    IntervalI0 = 0x20,
    /// Load interval endpoint i1
    ///
    /// Format: `dst:reg`
    IntervalI1 = 0x21,
    /// Interval meet: i ∧ j
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    IntervalMeet = 0x22,
    /// Interval join: i ∨ j
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    IntervalJoin = 0x23,
    /// Interval reversal: 1 - i
    ///
    /// Format: `dst:reg, src:reg`
    IntervalRev = 0x24,

    // ========================================================================
    // Univalence (0x30-0x3F)
    // ========================================================================
    /// Computational univalence: ua(equiv) — equivalence → type path
    ///
    /// Format: `dst:reg, equiv:reg`
    Ua = 0x30,
    /// Univalence inverse: ua_inv(path) — type path → equivalence
    ///
    /// Format: `dst:reg, path:reg`
    UaInv = 0x31,
    /// Equiv forward: e.forward(x)
    ///
    /// Format: `dst:reg, equiv:reg, value:reg`
    EquivFwd = 0x32,
    /// Equiv backward: e.inverse(x)
    ///
    /// Format: `dst:reg, equiv:reg, value:reg`
    EquivBwd = 0x33,
}

impl CubicalSubOpcode {
    /// Creates a cubical sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Path Construction
            0x00 => Some(Self::PathRefl),
            0x01 => Some(Self::PathLambda),
            0x02 => Some(Self::PathApp),
            0x03 => Some(Self::PathSym),
            0x04 => Some(Self::PathTrans),
            0x05 => Some(Self::PathAp),
            // Transport and Composition
            0x10 => Some(Self::Transport),
            0x11 => Some(Self::Hcomp),
            // Interval Operations
            0x20 => Some(Self::IntervalI0),
            0x21 => Some(Self::IntervalI1),
            0x22 => Some(Self::IntervalMeet),
            0x23 => Some(Self::IntervalJoin),
            0x24 => Some(Self::IntervalRev),
            // Univalence
            0x30 => Some(Self::Ua),
            0x31 => Some(Self::UaInv),
            0x32 => Some(Self::EquivFwd),
            0x33 => Some(Self::EquivBwd),
            _ => None,
        }
    }

    /// Returns the byte value of this cubical sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this cubical sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Path Construction
            Self::PathRefl => "CUB_PATH_REFL",
            Self::PathLambda => "CUB_PATH_LAMBDA",
            Self::PathApp => "CUB_PATH_APP",
            Self::PathSym => "CUB_PATH_SYM",
            Self::PathTrans => "CUB_PATH_TRANS",
            Self::PathAp => "CUB_PATH_AP",
            // Transport and Composition
            Self::Transport => "CUB_TRANSPORT",
            Self::Hcomp => "CUB_HCOMP",
            // Interval Operations
            Self::IntervalI0 => "CUB_INTERVAL_I0",
            Self::IntervalI1 => "CUB_INTERVAL_I1",
            Self::IntervalMeet => "CUB_INTERVAL_MEET",
            Self::IntervalJoin => "CUB_INTERVAL_JOIN",
            Self::IntervalRev => "CUB_INTERVAL_REV",
            // Univalence
            Self::Ua => "CUB_UA",
            Self::UaInv => "CUB_UA_INV",
            Self::EquivFwd => "CUB_EQUIV_FWD",
            Self::EquivBwd => "CUB_EQUIV_BWD",
        }
    }

    /// Returns the category of this cubical sub-opcode.
    pub fn category(self) -> &'static str {
        match self as u8 {
            0x00..=0x0F => "Path Construction",
            0x10..=0x1F => "Transport and Composition",
            0x20..=0x2F => "Interval Operations",
            0x30..=0x3F => "Univalence",
            _ => "Unknown",
        }
    }
}

// ============================================================================
// GPU Extended Sub-Opcodes
// ============================================================================

/// GPU extended sub-opcodes for use with `GpuExtended` (0xF8) prefix.
///
/// This provides a comprehensive GPU instruction set for:
/// - Kernel execution and cooperative launches
/// - Stream management with priorities
/// - Event-based synchronization and profiling
/// - Advanced memory operations (async, pinned, managed)
/// - Multi-GPU device management and peer access
///
/// # Encoding
///
/// ```text
/// [0xF8] [sub_opcode:u8] [operands...]
/// ```
///
/// # Example
///
/// ```text
/// // Create stream with priority
/// GpuExtended StreamCreateWithPriority dst:r0, priority:r1
///
/// // Async memcpy on stream
/// GpuExtended MemcpyAsync dst:r2, src:r3, size:r4, stream:r0
///
/// // Record event
/// GpuExtended EventRecord event:r5, stream:r0
///
/// // Elapsed time between events
/// GpuExtended EventElapsed dst:r6, start:r5, end:r7
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum GpuSubOpcode {
    // ========================================================================
    // Kernel Execution (0x00-0x0F)
    // ========================================================================
    /// Launch GPU kernel on stream.
    ///
    /// Format: `kernel_id:u32, grid:[reg;3], block:[reg;3], shared_mem:reg, stream:reg, args:vec<reg>`
    Launch = 0x00,

    /// Cooperative kernel launch with grid-wide synchronization.
    ///
    /// Enables `__syncthreads()` across entire grid. Limited by device occupancy.
    /// Format: `kernel_id:u32, grid:[reg;3], block:[reg;3], shared_mem:reg, stream:reg, args:vec<reg>`
    LaunchCooperative = 0x01,

    /// Launch kernel on multiple devices simultaneously.
    ///
    /// Format: `kernel_id:u32, device_count:u8, configs:vec<LaunchConfig>`
    LaunchMultiDevice = 0x02,

    // ========================================================================
    // Synchronization (0x10-0x1F)
    // ========================================================================
    /// Synchronize specific stream.
    ///
    /// Format: `stream:reg`
    SyncStream = 0x10,

    /// Synchronize entire device (all streams).
    ///
    /// Format: (no operands)
    SyncDevice = 0x11,

    /// Wait for event to complete.
    ///
    /// Format: `event:reg`
    SyncEvent = 0x12,

    /// Check if stream is idle (non-blocking).
    ///
    /// Format: `dst:reg, stream:reg`
    QueryStream = 0x13,

    // ========================================================================
    // Memory Operations (0x20-0x2F)
    // ========================================================================
    /// Synchronous memory copy.
    ///
    /// Format: `dst:reg, src:reg, size:reg, direction:u8`
    /// Direction: 0=H2D, 1=D2H, 2=D2D, 3=H2H
    Memcpy = 0x20,

    /// Asynchronous memory copy on stream.
    ///
    /// Format: `dst:reg, src:reg, size:reg, direction:u8, stream:reg`
    MemcpyAsync = 0x21,

    /// Allocate device memory.
    ///
    /// Format: `dst:reg, size:reg, device:reg`
    Alloc = 0x22,

    /// Free device memory.
    ///
    /// Format: `ptr:reg`
    Free = 0x23,

    /// Pin host memory for faster transfers.
    ///
    /// Format: `ptr:reg, size:reg`
    PinMemory = 0x24,

    /// Unpin previously pinned host memory.
    ///
    /// Format: `ptr:reg`
    UnpinMemory = 0x25,

    /// Prefetch memory to device.
    ///
    /// Format: `ptr:reg, size:reg, device:reg, stream:reg`
    Prefetch = 0x26,

    /// Set memory to value (synchronous).
    ///
    /// Format: `ptr:reg, value:u8, size:reg`
    Memset = 0x27,

    /// Set memory to value (asynchronous).
    ///
    /// Format: `ptr:reg, value:u8, size:reg, stream:reg`
    MemsetAsync = 0x28,

    /// 2D memory copy for pitched allocations.
    ///
    /// Format: `dst:reg, dst_pitch:reg, src:reg, src_pitch:reg, width:reg, height:reg, direction:u8`
    Memcpy2D = 0x29,

    /// 2D async memory copy.
    ///
    /// Format: `dst:reg, dst_pitch:reg, src:reg, src_pitch:reg, width:reg, height:reg, direction:u8, stream:reg`
    Memcpy2DAsync = 0x2A,

    /// Host-to-device memory copy (synchronous).
    ///
    /// Format: `dst:reg, src:reg, size:reg`
    /// Semantic: Copy `size` bytes from host pointer `src` to device pointer `dst`.
    MemcpyH2D = 0x2B,

    /// Device-to-host memory copy (synchronous).
    ///
    /// Format: `dst:reg, src:reg, size:reg`
    /// Semantic: Copy `size` bytes from device pointer `src` to host pointer `dst`.
    MemcpyD2H = 0x2C,

    /// Device-to-device memory copy (synchronous).
    ///
    /// Format: `dst:reg, src:reg, size:reg`
    /// Semantic: Copy `size` bytes from device pointer `src` to device pointer `dst`.
    MemcpyD2D = 0x2D,

    /// Host-to-device async memory copy.
    ///
    /// Format: `dst:reg, src:reg, size:reg, stream:reg`
    MemcpyAsyncH2D = 0x2E,

    /// Device-to-host async memory copy.
    ///
    /// Format: `dst:reg, src:reg, size:reg, stream:reg`
    MemcpyAsyncD2H = 0x2F,

    // ========================================================================
    // Stream Management (0x30-0x3F)
    // ========================================================================
    /// Create new stream.
    ///
    /// Format: `dst:reg`
    StreamCreate = 0x30,

    /// Destroy stream.
    ///
    /// Format: `stream:reg`
    StreamDestroy = 0x31,

    /// Query stream completion status (non-blocking).
    ///
    /// Format: `dst:reg, stream:reg`
    /// Returns: 1 if complete, 0 if still executing
    StreamQuery = 0x32,

    /// Make stream wait for event.
    ///
    /// Format: `stream:reg, event:reg`
    StreamWaitEvent = 0x33,

    /// Create stream with priority.
    ///
    /// Format: `dst:reg, priority:reg`
    /// Priority: lower number = higher priority
    StreamCreateWithPriority = 0x34,

    /// Get stream priority.
    ///
    /// Format: `dst:reg, stream:reg`
    StreamGetPriority = 0x35,

    /// Create non-blocking stream.
    ///
    /// Format: `dst:reg`
    StreamCreateNonBlocking = 0x36,

    /// Add callback to stream.
    ///
    /// Format: `stream:reg, callback_id:u32, user_data:reg`
    StreamAddCallback = 0x37,

    // ========================================================================
    // Event Management (0x40-0x4F)
    // ========================================================================
    /// Create event.
    ///
    /// Format: `dst:reg`
    EventCreate = 0x40,

    /// Destroy event.
    ///
    /// Format: `event:reg`
    EventDestroy = 0x41,

    /// Record event on stream.
    ///
    /// Format: `event:reg, stream:reg`
    EventRecord = 0x42,

    /// Synchronize on event (blocking).
    ///
    /// Format: `event:reg`
    EventSynchronize = 0x43,

    /// Query event status (non-blocking).
    ///
    /// Format: `dst:reg, event:reg`
    /// Returns: 1 if recorded event completed, 0 otherwise
    EventQuery = 0x44,

    /// Compute elapsed time between events (milliseconds).
    ///
    /// Format: `dst:reg, start_event:reg, end_event:reg`
    EventElapsed = 0x45,

    /// Create event with flags.
    ///
    /// Format: `dst:reg, flags:u8`
    /// Flags: 0x01=BlockingSync, 0x02=DisableTiming, 0x04=Interprocess
    EventCreateWithFlags = 0x46,

    /// Record event with flags.
    ///
    /// Format: `event:reg, stream:reg, flags:u8`
    EventRecordWithFlags = 0x47,

    // ========================================================================
    // Device Management (0x50-0x5F)
    // ========================================================================
    /// Get current device ID.
    ///
    /// Format: `dst:reg`
    GetDevice = 0x50,

    /// Set current device.
    ///
    /// Format: `device:reg`
    SetDevice = 0x51,

    /// Get device count.
    ///
    /// Format: `dst:reg`
    GetDeviceCount = 0x52,

    /// Get device properties.
    ///
    /// Format: `dst:reg, device:reg, prop_id:u8`
    /// prop_id: 0=name, 1=compute_cap, 2=multiprocessors, 3=max_threads,
    ///          4=warp_size, 5=global_mem, 6=shared_mem, 7=max_blocks
    GetDeviceProperty = 0x53,

    /// Get device memory info.
    ///
    /// Format: `free:reg, total:reg, device:reg`
    GetMemoryInfo = 0x54,

    /// Check if device can access peer memory.
    ///
    /// Format: `dst:reg, device:reg, peer_device:reg`
    CanAccessPeer = 0x55,

    /// Enable peer memory access.
    ///
    /// Format: `device:reg, peer_device:reg`
    EnablePeerAccess = 0x56,

    /// Disable peer memory access.
    ///
    /// Format: `device:reg, peer_device:reg`
    DisablePeerAccess = 0x57,

    /// Reset device (free all allocations).
    ///
    /// Format: `device:reg`
    DeviceReset = 0x58,

    /// Set device flags.
    ///
    /// Format: `flags:u8`
    /// Flags: 0x01=ScheduleSpin, 0x02=ScheduleYield, 0x04=ScheduleBlocking
    SetDeviceFlags = 0x59,

    // ========================================================================
    // Unified/Managed Memory (0x60-0x6F)
    // ========================================================================
    /// Allocate managed (unified) memory.
    ///
    /// Format: `dst:reg, size:reg, flags:u8`
    /// Flags: 0x01=AttachGlobal, 0x02=AttachHost
    MallocManaged = 0x60,

    /// Set memory advise hint.
    ///
    /// Format: `ptr:reg, size:reg, advice:u8, device:reg`
    /// Advice: 0=SetReadMostly, 1=UnsetReadMostly, 2=SetPreferredLocation,
    ///         3=UnsetPreferredLocation, 4=SetAccessedBy, 5=UnsetAccessedBy
    MemAdvise = 0x61,

    /// Prefetch managed memory asynchronously.
    ///
    /// Format: `ptr:reg, size:reg, device:reg, stream:reg`
    PrefetchAsync = 0x62,

    /// Get memory attributes.
    ///
    /// Format: `dst:reg, ptr:reg, attr_id:u8`
    /// attr_id: 0=type, 1=device, 2=is_managed
    MemGetAttribute = 0x63,

    // ========================================================================
    // Graph API (0x70-0x7F) - For CUDA Graphs / Metal ICB
    // ========================================================================
    /// Create execution graph.
    ///
    /// Format: `dst:reg`
    GraphCreate = 0x70,

    /// Begin graph capture on stream.
    ///
    /// Format: `graph:reg, stream:reg, mode:u8`
    /// Mode: 0=Global, 1=ThreadLocal, 2=Relaxed
    GraphBeginCapture = 0x71,

    /// End graph capture.
    ///
    /// Format: `graph:reg, stream:reg`
    GraphEndCapture = 0x72,

    /// Instantiate graph for execution.
    ///
    /// Format: `dst:reg, graph:reg`
    GraphInstantiate = 0x73,

    /// Launch instantiated graph.
    ///
    /// Format: `graph_exec:reg, stream:reg`
    GraphLaunch = 0x74,

    /// Destroy graph.
    ///
    /// Format: `graph:reg`
    GraphDestroy = 0x75,

    /// Destroy graph executable.
    ///
    /// Format: `graph_exec:reg`
    GraphExecDestroy = 0x76,

    /// Update graph executable with new graph.
    ///
    /// Format: `graph_exec:reg, graph:reg`
    GraphExecUpdate = 0x77,

    // ========================================================================
    // Profiling (0x80-0x8F)
    // ========================================================================
    /// Start profiling range.
    ///
    /// Format: `name_id:u32`
    ProfileRangeStart = 0x80,

    /// End profiling range.
    ///
    /// Format: (no operands)
    ProfileRangeEnd = 0x81,

    /// Push profiling marker.
    ///
    /// Format: `name_id:u32`
    ProfileMarkerPush = 0x82,

    /// Pop profiling marker.
    ///
    /// Format: (no operands)
    ProfileMarkerPop = 0x83,

    // ========================================================================
    // Device Enumeration (0x90-0x9F)
    // ========================================================================
    /// Enumerate available CUDA devices.
    ///
    /// Format: `dst:reg`
    /// Returns: List of device IDs available for CUDA backend
    EnumerateCuda = 0x90,

    /// Enumerate available Metal devices.
    ///
    /// Format: `dst:reg`
    /// Returns: List of device IDs available for Metal backend
    EnumerateMetal = 0x91,

    /// Enumerate available ROCm devices.
    ///
    /// Format: `dst:reg`
    /// Returns: List of device IDs available for ROCm backend
    EnumerateRocm = 0x92,

    /// Enumerate available Vulkan devices.
    ///
    /// Format: `dst:reg`
    /// Returns: List of device IDs available for Vulkan backend
    EnumerateVulkan = 0x93,

    // ========================================================================
    // Thread Intrinsics (0xA0-0xAF) - CPU-Fallback GPU Thread Model
    // ========================================================================
    /// Get thread index X within the current block.
    ///
    /// Format: `dst:reg`
    /// Returns: threadIdx.x (u32 as i64)
    ThreadIdX = 0xA0,

    /// Get thread index Y within the current block.
    ///
    /// Format: `dst:reg`
    ThreadIdY = 0xA1,

    /// Get thread index Z within the current block.
    ///
    /// Format: `dst:reg`
    ThreadIdZ = 0xA2,

    /// Get block index X within the grid.
    ///
    /// Format: `dst:reg`
    BlockIdX = 0xA3,

    /// Get block index Y within the grid.
    ///
    /// Format: `dst:reg`
    BlockIdY = 0xA4,

    /// Get block index Z within the grid.
    ///
    /// Format: `dst:reg`
    BlockIdZ = 0xA5,

    /// Get block dimension X (number of threads per block in X).
    ///
    /// Format: `dst:reg`
    BlockDimX = 0xA6,

    /// Get block dimension Y.
    ///
    /// Format: `dst:reg`
    BlockDimY = 0xA7,

    /// Get block dimension Z.
    ///
    /// Format: `dst:reg`
    BlockDimZ = 0xA8,

    /// Get grid dimension X (number of blocks in X).
    ///
    /// Format: `dst:reg`
    GridDimX = 0xA9,

    /// Get grid dimension Y.
    ///
    /// Format: `dst:reg`
    GridDimY = 0xAA,

    /// Get grid dimension Z.
    ///
    /// Format: `dst:reg`
    GridDimZ = 0xAB,

    /// Block-level barrier synchronization (__syncthreads).
    ///
    /// Format: (no operands)
    /// In CPU fallback: no-op (threads execute sequentially within a block).
    SyncThreads = 0xAC,

    /// Warp-level barrier synchronization (__syncwarp).
    ///
    /// Format: `mask:reg` (optional, defaults to full warp mask)
    /// In CPU fallback: no-op.
    SyncWarp = 0xAD,

    /// Get warp size.
    ///
    /// Format: `dst:reg`
    /// Returns: 32 (standard warp size for CPU simulation)
    WarpSize = 0xAE,

    /// Get linear thread ID (threadIdx.x + threadIdx.y * blockDim.x + ...).
    ///
    /// Format: `dst:reg`
    LinearThreadId = 0xAF,

    // ========================================================================
    // Shared Memory Operations (0xB0-0xBF)
    // ========================================================================
    /// Allocate shared memory (returns base offset).
    ///
    /// Format: `dst:reg, size:reg`
    /// Returns: byte offset into shared memory block
    SharedMemAlloc = 0xB0,

    /// Load i64 from shared memory.
    ///
    /// Format: `dst:reg, offset:reg`
    SharedMemLoadI64 = 0xB1,

    /// Store i64 to shared memory.
    ///
    /// Format: `offset:reg, value:reg`
    SharedMemStoreI64 = 0xB2,

    /// Load f64 from shared memory.
    ///
    /// Format: `dst:reg, offset:reg`
    SharedMemLoadF64 = 0xB3,

    /// Store f64 to shared memory.
    ///
    /// Format: `offset:reg, value:reg`
    SharedMemStoreF64 = 0xB4,

    /// Atomic add on shared memory i64.
    ///
    /// Format: `dst:reg, offset:reg, value:reg`
    /// Returns: previous value
    SharedMemAtomicAddI64 = 0xB5,

    /// Atomic add on shared memory f64.
    ///
    /// Format: `dst:reg, offset:reg, value:reg`
    /// Returns: previous value
    SharedMemAtomicAddF64 = 0xB6,

    /// Atomic CAS on shared memory i64.
    ///
    /// Format: `dst:reg, offset:reg, expected:reg, desired:reg`
    /// Returns: previous value
    SharedMemAtomicCasI64 = 0xB7,

    /// Atomic max on shared memory i64.
    ///
    /// Format: `dst:reg, offset:reg, value:reg`
    SharedMemAtomicMaxI64 = 0xB8,

    /// Atomic min on shared memory i64.
    ///
    /// Format: `dst:reg, offset:reg, value:reg`
    SharedMemAtomicMinI64 = 0xB9,

    /// Load u32 from shared memory.
    ///
    /// Format: `dst:reg, offset:reg`
    SharedMemLoadU32 = 0xBA,

    /// Store u32 to shared memory.
    ///
    /// Format: `offset:reg, value:reg`
    SharedMemStoreU32 = 0xBB,
}

impl GpuSubOpcode {
    /// Creates a GPU sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Kernel Execution
            0x00 => Some(Self::Launch),
            0x01 => Some(Self::LaunchCooperative),
            0x02 => Some(Self::LaunchMultiDevice),
            // Synchronization
            0x10 => Some(Self::SyncStream),
            0x11 => Some(Self::SyncDevice),
            0x12 => Some(Self::SyncEvent),
            0x13 => Some(Self::QueryStream),
            // Memory Operations
            0x20 => Some(Self::Memcpy),
            0x21 => Some(Self::MemcpyAsync),
            0x22 => Some(Self::Alloc),
            0x23 => Some(Self::Free),
            0x24 => Some(Self::PinMemory),
            0x25 => Some(Self::UnpinMemory),
            0x26 => Some(Self::Prefetch),
            0x27 => Some(Self::Memset),
            0x28 => Some(Self::MemsetAsync),
            0x29 => Some(Self::Memcpy2D),
            0x2A => Some(Self::Memcpy2DAsync),
            0x2B => Some(Self::MemcpyH2D),
            0x2C => Some(Self::MemcpyD2H),
            0x2D => Some(Self::MemcpyD2D),
            0x2E => Some(Self::MemcpyAsyncH2D),
            0x2F => Some(Self::MemcpyAsyncD2H),
            // Stream Management
            0x30 => Some(Self::StreamCreate),
            0x31 => Some(Self::StreamDestroy),
            0x32 => Some(Self::StreamQuery),
            0x33 => Some(Self::StreamWaitEvent),
            0x34 => Some(Self::StreamCreateWithPriority),
            0x35 => Some(Self::StreamGetPriority),
            0x36 => Some(Self::StreamCreateNonBlocking),
            0x37 => Some(Self::StreamAddCallback),
            // Event Management
            0x40 => Some(Self::EventCreate),
            0x41 => Some(Self::EventDestroy),
            0x42 => Some(Self::EventRecord),
            0x43 => Some(Self::EventSynchronize),
            0x44 => Some(Self::EventQuery),
            0x45 => Some(Self::EventElapsed),
            0x46 => Some(Self::EventCreateWithFlags),
            0x47 => Some(Self::EventRecordWithFlags),
            // Device Management
            0x50 => Some(Self::GetDevice),
            0x51 => Some(Self::SetDevice),
            0x52 => Some(Self::GetDeviceCount),
            0x53 => Some(Self::GetDeviceProperty),
            0x54 => Some(Self::GetMemoryInfo),
            0x55 => Some(Self::CanAccessPeer),
            0x56 => Some(Self::EnablePeerAccess),
            0x57 => Some(Self::DisablePeerAccess),
            0x58 => Some(Self::DeviceReset),
            0x59 => Some(Self::SetDeviceFlags),
            // Unified/Managed Memory
            0x60 => Some(Self::MallocManaged),
            0x61 => Some(Self::MemAdvise),
            0x62 => Some(Self::PrefetchAsync),
            0x63 => Some(Self::MemGetAttribute),
            // Graph API
            0x70 => Some(Self::GraphCreate),
            0x71 => Some(Self::GraphBeginCapture),
            0x72 => Some(Self::GraphEndCapture),
            0x73 => Some(Self::GraphInstantiate),
            0x74 => Some(Self::GraphLaunch),
            0x75 => Some(Self::GraphDestroy),
            0x76 => Some(Self::GraphExecDestroy),
            0x77 => Some(Self::GraphExecUpdate),
            // Profiling
            0x80 => Some(Self::ProfileRangeStart),
            0x81 => Some(Self::ProfileRangeEnd),
            0x82 => Some(Self::ProfileMarkerPush),
            0x83 => Some(Self::ProfileMarkerPop),
            // Device Enumeration
            0x90 => Some(Self::EnumerateCuda),
            0x91 => Some(Self::EnumerateMetal),
            0x92 => Some(Self::EnumerateRocm),
            0x93 => Some(Self::EnumerateVulkan),
            // Thread Intrinsics
            0xA0 => Some(Self::ThreadIdX),
            0xA1 => Some(Self::ThreadIdY),
            0xA2 => Some(Self::ThreadIdZ),
            0xA3 => Some(Self::BlockIdX),
            0xA4 => Some(Self::BlockIdY),
            0xA5 => Some(Self::BlockIdZ),
            0xA6 => Some(Self::BlockDimX),
            0xA7 => Some(Self::BlockDimY),
            0xA8 => Some(Self::BlockDimZ),
            0xA9 => Some(Self::GridDimX),
            0xAA => Some(Self::GridDimY),
            0xAB => Some(Self::GridDimZ),
            0xAC => Some(Self::SyncThreads),
            0xAD => Some(Self::SyncWarp),
            0xAE => Some(Self::WarpSize),
            0xAF => Some(Self::LinearThreadId),
            // Shared Memory Operations
            0xB0 => Some(Self::SharedMemAlloc),
            0xB1 => Some(Self::SharedMemLoadI64),
            0xB2 => Some(Self::SharedMemStoreI64),
            0xB3 => Some(Self::SharedMemLoadF64),
            0xB4 => Some(Self::SharedMemStoreF64),
            0xB5 => Some(Self::SharedMemAtomicAddI64),
            0xB6 => Some(Self::SharedMemAtomicAddF64),
            0xB7 => Some(Self::SharedMemAtomicCasI64),
            0xB8 => Some(Self::SharedMemAtomicMaxI64),
            0xB9 => Some(Self::SharedMemAtomicMinI64),
            0xBA => Some(Self::SharedMemLoadU32),
            0xBB => Some(Self::SharedMemStoreU32),
            _ => None,
        }
    }

    /// Returns the byte value of this GPU sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this GPU sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Kernel Execution
            Self::Launch => "GPU_LAUNCH",
            Self::LaunchCooperative => "GPU_LAUNCH_COOP",
            Self::LaunchMultiDevice => "GPU_LAUNCH_MULTI",
            // Synchronization
            Self::SyncStream => "GPU_SYNC_STREAM",
            Self::SyncDevice => "GPU_SYNC_DEVICE",
            Self::SyncEvent => "GPU_SYNC_EVENT",
            Self::QueryStream => "GPU_QUERY_STREAM",
            // Memory Operations
            Self::Memcpy => "GPU_MEMCPY",
            Self::MemcpyAsync => "GPU_MEMCPY_ASYNC",
            Self::Alloc => "GPU_ALLOC",
            Self::Free => "GPU_FREE",
            Self::PinMemory => "GPU_PIN",
            Self::UnpinMemory => "GPU_UNPIN",
            Self::Prefetch => "GPU_PREFETCH",
            Self::Memset => "GPU_MEMSET",
            Self::MemsetAsync => "GPU_MEMSET_ASYNC",
            Self::Memcpy2D => "GPU_MEMCPY_2D",
            Self::Memcpy2DAsync => "GPU_MEMCPY_2D_ASYNC",
            Self::MemcpyH2D => "GPU_MEMCPY_H2D",
            Self::MemcpyD2H => "GPU_MEMCPY_D2H",
            Self::MemcpyD2D => "GPU_MEMCPY_D2D",
            Self::MemcpyAsyncH2D => "GPU_MEMCPY_ASYNC_H2D",
            Self::MemcpyAsyncD2H => "GPU_MEMCPY_ASYNC_D2H",
            // Stream Management
            Self::StreamCreate => "GPU_STREAM_CREATE",
            Self::StreamDestroy => "GPU_STREAM_DESTROY",
            Self::StreamQuery => "GPU_STREAM_QUERY",
            Self::StreamWaitEvent => "GPU_STREAM_WAIT_EVENT",
            Self::StreamCreateWithPriority => "GPU_STREAM_CREATE_PRIO",
            Self::StreamGetPriority => "GPU_STREAM_GET_PRIO",
            Self::StreamCreateNonBlocking => "GPU_STREAM_CREATE_NB",
            Self::StreamAddCallback => "GPU_STREAM_CALLBACK",
            // Event Management
            Self::EventCreate => "GPU_EVENT_CREATE",
            Self::EventDestroy => "GPU_EVENT_DESTROY",
            Self::EventRecord => "GPU_EVENT_RECORD",
            Self::EventSynchronize => "GPU_EVENT_SYNC",
            Self::EventQuery => "GPU_EVENT_QUERY",
            Self::EventElapsed => "GPU_EVENT_ELAPSED",
            Self::EventCreateWithFlags => "GPU_EVENT_CREATE_F",
            Self::EventRecordWithFlags => "GPU_EVENT_RECORD_F",
            // Device Management
            Self::GetDevice => "GPU_GET_DEVICE",
            Self::SetDevice => "GPU_SET_DEVICE",
            Self::GetDeviceCount => "GPU_DEVICE_COUNT",
            Self::GetDeviceProperty => "GPU_DEVICE_PROP",
            Self::GetMemoryInfo => "GPU_MEM_INFO",
            Self::CanAccessPeer => "GPU_CAN_PEER",
            Self::EnablePeerAccess => "GPU_ENABLE_PEER",
            Self::DisablePeerAccess => "GPU_DISABLE_PEER",
            Self::DeviceReset => "GPU_DEVICE_RESET",
            Self::SetDeviceFlags => "GPU_SET_FLAGS",
            // Unified/Managed Memory
            Self::MallocManaged => "GPU_MALLOC_MANAGED",
            Self::MemAdvise => "GPU_MEM_ADVISE",
            Self::PrefetchAsync => "GPU_PREFETCH_ASYNC",
            Self::MemGetAttribute => "GPU_MEM_ATTR",
            // Graph API
            Self::GraphCreate => "GPU_GRAPH_CREATE",
            Self::GraphBeginCapture => "GPU_GRAPH_BEGIN",
            Self::GraphEndCapture => "GPU_GRAPH_END",
            Self::GraphInstantiate => "GPU_GRAPH_INST",
            Self::GraphLaunch => "GPU_GRAPH_LAUNCH",
            Self::GraphDestroy => "GPU_GRAPH_DESTROY",
            Self::GraphExecDestroy => "GPU_GRAPH_EXEC_DESTROY",
            Self::GraphExecUpdate => "GPU_GRAPH_EXEC_UPDATE",
            // Profiling
            Self::ProfileRangeStart => "GPU_PROF_START",
            Self::ProfileRangeEnd => "GPU_PROF_END",
            Self::ProfileMarkerPush => "GPU_MARKER_PUSH",
            Self::ProfileMarkerPop => "GPU_MARKER_POP",
            // Device Enumeration
            Self::EnumerateCuda => "GPU_ENUM_CUDA",
            Self::EnumerateMetal => "GPU_ENUM_METAL",
            Self::EnumerateRocm => "GPU_ENUM_ROCM",
            Self::EnumerateVulkan => "GPU_ENUM_VULKAN",
            // Thread Intrinsics
            Self::ThreadIdX => "GPU_THREAD_ID_X",
            Self::ThreadIdY => "GPU_THREAD_ID_Y",
            Self::ThreadIdZ => "GPU_THREAD_ID_Z",
            Self::BlockIdX => "GPU_BLOCK_ID_X",
            Self::BlockIdY => "GPU_BLOCK_ID_Y",
            Self::BlockIdZ => "GPU_BLOCK_ID_Z",
            Self::BlockDimX => "GPU_BLOCK_DIM_X",
            Self::BlockDimY => "GPU_BLOCK_DIM_Y",
            Self::BlockDimZ => "GPU_BLOCK_DIM_Z",
            Self::GridDimX => "GPU_GRID_DIM_X",
            Self::GridDimY => "GPU_GRID_DIM_Y",
            Self::GridDimZ => "GPU_GRID_DIM_Z",
            Self::SyncThreads => "GPU_SYNC_THREADS",
            Self::SyncWarp => "GPU_SYNC_WARP",
            Self::WarpSize => "GPU_WARP_SIZE",
            Self::LinearThreadId => "GPU_LINEAR_THREAD_ID",
            // Shared Memory Operations
            Self::SharedMemAlloc => "GPU_SMEM_ALLOC",
            Self::SharedMemLoadI64 => "GPU_SMEM_LOAD_I64",
            Self::SharedMemStoreI64 => "GPU_SMEM_STORE_I64",
            Self::SharedMemLoadF64 => "GPU_SMEM_LOAD_F64",
            Self::SharedMemStoreF64 => "GPU_SMEM_STORE_F64",
            Self::SharedMemAtomicAddI64 => "GPU_SMEM_ATOMIC_ADD_I64",
            Self::SharedMemAtomicAddF64 => "GPU_SMEM_ATOMIC_ADD_F64",
            Self::SharedMemAtomicCasI64 => "GPU_SMEM_ATOMIC_CAS_I64",
            Self::SharedMemAtomicMaxI64 => "GPU_SMEM_ATOMIC_MAX_I64",
            Self::SharedMemAtomicMinI64 => "GPU_SMEM_ATOMIC_MIN_I64",
            Self::SharedMemLoadU32 => "GPU_SMEM_LOAD_U32",
            Self::SharedMemStoreU32 => "GPU_SMEM_STORE_U32",
        }
    }

    /// Returns the category of this GPU sub-opcode.
    pub fn category(self) -> &'static str {
        match self as u8 {
            0x00..=0x0F => "Kernel Execution",
            0x10..=0x1F => "Synchronization",
            0x20..=0x2F => "Memory Operations",
            0x30..=0x3F => "Stream Management",
            0x40..=0x4F => "Event Management",
            0x50..=0x5F => "Device Management",
            0x60..=0x6F => "Unified Memory",
            0x70..=0x7F => "Graph API",
            0x80..=0x8F => "Profiling",
            0x90..=0x9F => "Device Enumeration",
            0xA0..=0xAF => "Thread Intrinsics",
            0xB0..=0xBF => "Shared Memory",
            _ => "Unknown",
        }
    }

    /// Returns true if this operation requires a stream.
    pub fn requires_stream(self) -> bool {
        matches!(
            self,
            Self::Launch
                | Self::LaunchCooperative
                | Self::MemcpyAsync
                | Self::MemsetAsync
                | Self::Memcpy2DAsync
                | Self::StreamWaitEvent
                | Self::EventRecord
                | Self::EventRecordWithFlags
                | Self::Prefetch
                | Self::PrefetchAsync
                | Self::GraphBeginCapture
                | Self::GraphEndCapture
                | Self::GraphLaunch
                | Self::StreamAddCallback
        )
    }

    /// Returns true if this operation is a synchronization operation.
    pub fn is_sync(self) -> bool {
        matches!(
            self,
            Self::SyncStream | Self::SyncDevice | Self::SyncEvent | Self::EventSynchronize
        )
    }

    /// Returns true if this operation allocates resources.
    pub fn allocates(self) -> bool {
        matches!(
            self,
            Self::Alloc
                | Self::MallocManaged
                | Self::StreamCreate
                | Self::StreamCreateWithPriority
                | Self::StreamCreateNonBlocking
                | Self::EventCreate
                | Self::EventCreateWithFlags
                | Self::GraphCreate
                | Self::GraphInstantiate
        )
    }

    /// Returns true if this operation frees resources.
    pub fn deallocates(self) -> bool {
        matches!(
            self,
            Self::Free
                | Self::StreamDestroy
                | Self::EventDestroy
                | Self::GraphDestroy
                | Self::GraphExecDestroy
        )
    }
}

// ============================================================================
// Tensor Extended Sub-Opcodes
// ============================================================================

/// Tensor extended sub-opcodes for use with `TensorExtended` (0xFF) prefix.
///
/// This provides an extensible tensor instruction set for advanced operations:
/// - Pooling operations (max, avg, adaptive)
/// - Linear algebra decompositions (QR, SVD, LU, Cholesky)
/// - Eigenvalue/eigenvector computation
/// - Linear system solvers (general, least squares)
/// - Advanced indexing (gather, scatter, permute)
/// - Reduction variants (argmin)
///
/// # Encoding
///
/// ```text
/// [0xFF] [sub_opcode:u8] [operands...]
/// ```
///
/// # Example
///
/// ```text
/// // Pooling operation
/// TensorExtended Pool dst:r0, src:r1, op:max, kernel:[2,2], stride:[2,2], pad:[0,0]
///
/// // QR decomposition
/// TensorExtended QR q:r0, r:r1, src:r2, mode:reduced
///
/// // General linear solve
/// TensorExtended Solve dst:r0, a:r1, b:r2
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorSubOpcode {
    // ========================================================================
    // Pooling Operations (0x00-0x0F)
    // ========================================================================
    /// Pooling operation (max, avg, sum, adaptive).
    ///
    /// Format: `op:u8, dst:reg, src:reg, kernel_size:vec, stride:vec, padding:vec`
    Pool = 0x00,

    // ========================================================================
    // Register-based Tensor Operations (0x0D-0x0F, 0x13-0x1B)
    // These handle intrinsic calls where all arguments are registers.
    // Values 0x01-0x09 are RESERVED for TensorExtSubOpcode fallthrough.
    // ========================================================================

    /// Create zero tensor from register args.
    ///
    /// Format: `dst:reg, shape_reg:reg, dtype_reg:reg`
    NewFromArgs = 0x0D,

    /// Fill tensor from register args.
    ///
    /// Format: `dst:reg, shape_reg:reg, value_reg:reg, dtype_reg:reg`
    FillFromArgs = 0x0E,

    /// Create tensor from data+shape registers.
    ///
    /// Format: `dst:reg, data_reg:reg, shape_reg:reg, dtype_reg:reg`
    FromSliceArgs = 0x0F,

    // ========================================================================
    // Reduction Variants (0x10-0x1F)
    // ========================================================================
    /// Argmin along axis.
    ///
    /// Format: `dst:reg, src:reg, axis:i8, keepdim:bool`
    Argmin = 0x10,

    /// Nansum (sum ignoring NaN values).
    ///
    /// Format: `dst:reg, src:reg, axis:i8, keepdim:bool`
    Nansum = 0x11,

    /// Nanmean (mean ignoring NaN values).
    ///
    /// Format: `dst:reg, src:reg, axis:i8, keepdim:bool`
    Nanmean = 0x12,

    /// Element-wise binary op from register args.
    ///
    /// Format: `dst:reg, a_reg:reg, b_reg:reg, op_reg:reg`
    BinopFromArgs = 0x13,

    /// Element-wise unary op from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, op_reg:reg`
    UnopFromArgs = 0x14,

    /// Matrix multiply from register args.
    ///
    /// Format: `dst:reg, a_reg:reg, b_reg:reg`
    MatmulFromArgs = 0x15,

    /// Reduce from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, op_reg:reg, axis_reg:reg`
    ReduceFromArgs = 0x16,

    /// Reshape from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, shape_reg:reg`
    ReshapeFromArgs = 0x17,

    /// Transpose from register args.
    ///
    /// Format: `dst:reg, src_reg:reg`
    TransposeFromArgs = 0x18,

    /// Slice from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, ranges_reg:reg`
    SliceFromArgs = 0x19,

    /// Get element at flat index from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, index_reg:reg`
    GetElementFromArgs = 0x1A,

    /// Set element at flat index from register args.
    ///
    /// Format: `dst:reg, src_reg:reg, index_reg:reg, value_reg:reg`
    SetElementFromArgs = 0x1B,

    // ========================================================================
    // Advanced Indexing (0x20-0x2F)
    // ========================================================================
    /// Gather elements along axis using indices.
    ///
    /// Format: `dst:reg, src:reg, index:reg, axis:i8`
    Gather = 0x20,

    /// General axis permutation.
    ///
    /// Format: `dst:reg, src:reg, axes:vec<u8>`
    Permute = 0x21,

    /// Flip tensor along axes.
    ///
    /// Format: `dst:reg, src:reg, axes:vec<u8>`
    Flip = 0x22,

    /// Roll tensor along axis.
    ///
    /// Format: `dst:reg, src:reg, shift:i32, axis:i8`
    Roll = 0x23,

    // ========================================================================
    // Linear System Solvers (0x30-0x3F)
    // ========================================================================
    /// General linear system solve: A @ x = B.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    /// Solves for x given A and B matrices.
    Solve = 0x30,

    /// Least squares solve: minimize ||A @ x - B||.
    ///
    /// Format: `x:reg, residuals:reg, rank:reg, s:reg, a:reg, b:reg, rcond:f64`
    Lstsq = 0x31,

    /// Triangular solve with custom options.
    ///
    /// Format: `dst:reg, a:reg, b:reg, upper:bool, trans:bool, unit_diag:bool`
    TriSolve = 0x32,

    // ========================================================================
    // Matrix Decompositions (0x40-0x5F)
    // ========================================================================
    /// QR decomposition.
    ///
    /// Format: `q:reg, r:reg, src:reg, mode:u8`
    /// Mode: 0=reduced, 1=complete, 2=r_only
    QR = 0x40,

    /// Singular Value Decomposition.
    ///
    /// Format: `u:reg, s:reg, vh:reg, src:reg, full_matrices:bool, compute_uv:bool`
    SVD = 0x41,

    /// LU decomposition with pivoting.
    ///
    /// Format: `p:reg, l:reg, u:reg, src:reg`
    LU = 0x42,

    /// Eigenvalue decomposition (general).
    ///
    /// Format: `eigenvalues:reg, eigenvectors:reg, src:reg, compute_v:bool`
    Eig = 0x43,

    /// Symmetric/Hermitian eigenvalue decomposition.
    ///
    /// Format: `eigenvalues:reg, eigenvectors:reg, src:reg, upper:bool`
    EigSymmetric = 0x44,

    /// Schur decomposition.
    ///
    /// Format: `t:reg, z:reg, src:reg`
    Schur = 0x45,

    // ========================================================================
    // Matrix Properties (0x60-0x6F)
    // ========================================================================
    /// Matrix determinant.
    ///
    /// Format: `dst:reg, src:reg`
    Det = 0x60,

    /// Matrix rank.
    ///
    /// Format: `dst:reg, src:reg, tol:f64`
    Rank = 0x61,

    /// Matrix condition number.
    ///
    /// Format: `dst:reg, src:reg, p:u8`
    /// p: 1=1-norm, 2=2-norm (default), -1=inf-norm
    Cond = 0x62,

    /// Matrix trace.
    ///
    /// Format: `dst:reg, src:reg`
    Trace = 0x63,

    /// Matrix norm.
    ///
    /// Format: `dst:reg, src:reg, ord:i8`
    /// ord: -2=min singular, -1=min row sum, 0=Frobenius, 1=max row sum, 2=max singular
    Norm = 0x64,

    // ========================================================================
    // Advanced Operations (0x70-0x7F)
    // ========================================================================
    /// Kronecker product.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    Kron = 0x70,

    /// Cross product.
    ///
    /// Format: `dst:reg, a:reg, b:reg, axis:i8`
    Cross = 0x71,

    /// Tensor contraction.
    ///
    /// Format: `dst:reg, a:reg, b:reg, axes_a:vec, axes_b:vec`
    Contract = 0x72,

    /// Matrix power.
    ///
    /// Format: `dst:reg, src:reg, n:i32`
    MatrixPower = 0x73,

    /// Matrix exponential.
    ///
    /// Format: `dst:reg, src:reg`
    Expm = 0x74,

    /// Matrix logarithm.
    ///
    /// Format: `dst:reg, src:reg`
    Logm = 0x75,

    /// Matrix inverse.
    ///
    /// Format: `dst:reg, src:reg`
    Inverse = 0x76,

    /// Real FFT (real-to-complex).
    ///
    /// Format: `dst:reg, src:reg, n:u32`
    Rfft = 0x77,

    /// Inverse real FFT (complex-to-real).
    ///
    /// Format: `dst:reg, src:reg, n:u32`
    Irfft = 0x78,

    /// Complex multiplication.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ComplexMul = 0x79,

    /// Complex power.
    ///
    /// Format: `dst:reg, base:reg, exp:reg`
    ComplexPow = 0x7A,

    /// Parallel associative scan (SSM).
    ///
    /// Format: `dst:reg, op:u8, init:reg, elements:reg, dim:i8`
    SsmScan = 0x7B,

    /// Uniform random tensor.
    ///
    /// Format: `dst:reg, shape_len:u8, shape..., low:reg, high:reg`
    Uniform = 0x7C,

    /// Bincount (histogram binning).
    ///
    /// Format: `dst:reg, indices:reg, num_bins:u32`
    Bincount = 0x7D,

    /// N-dimensional gather.
    ///
    /// Format: `dst:reg, src:reg, indices:reg`
    GatherNd = 0x7E,

    /// Integer range tensor.
    ///
    /// Format: `dst:reg, start:reg, end:reg, step:reg`
    ArangeUsize = 0x7F,

    // ========================================================================
    // Extended Tensor Operations (0x80-0x8F)
    // ========================================================================

    /// Repeat tensor along new dimension.
    ///
    /// Format: `dst:reg, src:reg, times:u32`
    Repeat = 0x80,

    /// Element-wise hyperbolic tangent.
    ///
    /// Format: `dst:reg, src:reg`
    Tanh = 0x81,

    /// Sum all elements.
    ///
    /// Format: `dst:reg, src:reg`
    SumAll = 0x82,

    /// Create tensor from array.
    ///
    /// Format: `dst:reg, len:u32, values...`
    FromArray = 0x83,

    /// Check if in training mode.
    ///
    /// Format: `dst:reg`
    IsTraining = 0x84,

    /// Random float in [0, 1).
    ///
    /// Format: `dst:reg`
    RandomFloat01 = 0x85,

    /// Select elements from tensor using boolean mask.
    ///
    /// Format: `dst:reg, src:reg, mask:reg`
    MaskedSelect = 0x86,

    /// Leaky ReLU activation.
    ///
    /// Format: `dst:reg, src:reg, negative_slope:reg`
    LeakyRelu = 0x87,

    /// Extract diagonal from tensor or create diagonal tensor.
    ///
    /// Format: `dst:reg, src:reg, offset:i32`
    Diag = 0x88,

    /// Upper triangular matrix.
    ///
    /// Format: `dst:reg, src:reg, diagonal:i32`
    Triu = 0x89,

    /// Lower triangular matrix.
    ///
    /// Format: `dst:reg, src:reg, diagonal:i32`
    Tril = 0x8A,

    /// Indices of non-zero elements.
    ///
    /// Format: `dst:reg, src:reg`
    Nonzero = 0x8B,

    /// One-hot encoding.
    ///
    /// Format: `dst:reg, indices:reg, num_classes:u32`
    OneHot = 0x8C,

    /// Split tensor into chunks.
    ///
    /// Format: `dst:reg, src:reg, num_chunks:u32, dim:i8`
    Split = 0x8D,

    /// Split tensor at a specific index.
    ///
    /// Format: `dst_a:reg, dst_b:reg, src:reg, index:u32, dim:i8`
    SplitAt = 0x8E,

    /// Get scalar value from tensor at index.
    ///
    /// Format: `dst:reg, src:reg, indices:reg`
    GetScalar = 0x8F,

    // ========================================================================
    // Tokenizer Operations (0x90-0x9F)
    // ========================================================================

    /// Load BPE tokenizer from files.
    ///
    /// Format: `dst:reg, vocab_path:reg, merges_path:reg`
    TokenizerLoadBpe = 0x90,

    /// Load pretrained tokenizer by model name.
    ///
    /// Format: `dst:reg, model_name:reg`
    TokenizerLoadPretrained = 0x91,

    /// Encode text to tokens.
    ///
    /// Format: `dst:reg, tokenizer:reg, text:reg`
    TokenizerEncode = 0x92,

    /// Decode tokens to text.
    ///
    /// Format: `dst:reg, tokenizer:reg, tokens:reg`
    TokenizerDecode = 0x93,

    /// Load SentencePiece model.
    ///
    /// Format: `dst:reg, model_path:reg`
    TokenizerLoadSpm = 0x94,

    /// Encode with SentencePiece.
    ///
    /// Format: `dst:reg, tokenizer:reg, text:reg`
    TokenizerSpmEncode = 0x95,

    /// Decode with SentencePiece.
    ///
    /// Format: `dst:reg, tokenizer:reg, tokens:reg`
    TokenizerSpmDecode = 0x96,

    // ========================================================================
    // Sampling Operations (0xA0-0xAF)
    // ========================================================================

    /// Top-p (nucleus) sampling.
    ///
    /// Format: `dst:reg, logits:reg, p:reg`
    SampleTopP = 0xA0,

    /// Temperature sampling.
    ///
    /// Format: `dst:reg, logits:reg, temperature:reg`
    SampleTemperature = 0xA1,

    /// Paged attention for KV cache.
    ///
    /// Format: `dst:reg, q:reg, kv_cache:reg, block_table:reg, context_len:reg`
    PagedAttention = 0xA2,

    // ========================================================================
    // Inference Utility Operations (0xB0-0xBF)
    // ========================================================================

    /// Parse tool call from action string.
    ///
    /// Format: `dst:reg, action:reg`
    ParseToolCall = 0xB0,

    /// Format value for display.
    ///
    /// Format: `dst:reg, value:reg`
    FormatValue = 0xB1,

    /// Create tensor from USize slice.
    ///
    /// Format: `dst:reg, values:reg`
    TensorFromSliceUsize = 0xB2,

    /// Quantized matrix multiplication.
    ///
    /// Format: `dst:reg, input:reg, weight:reg, scale:reg, zero_point:reg`
    QuantizedMatmul = 0xB3,

    /// Tensor norm.
    ///
    /// Format: `dst:reg, x:reg`
    TensorNorm = 0xB4,

    /// Generate unique request ID.
    ///
    /// Format: `dst:reg`
    GenerateRequestId = 0xB5,

    /// Convert JSON schema to JSON.
    ///
    /// Format: `dst:reg, schema:reg`
    JsonSchemaToJson = 0xB6,

    /// Convert function schema to JSON.
    ///
    /// Format: `dst:reg, schema:reg`
    FunctionSchemaToJson = 0xB7,

    /// Parse function calls from response.
    ///
    /// Format: `dst:reg, response:reg`
    ParseFunctionCalls = 0xB8,

    // ========================================================================
    // Distributed/Collective Operations (0xC0-0xCF)
    // ========================================================================

    /// All-reduce: reduce tensor across all ranks and distribute result.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg, op:u8`
    /// Op: 0=Sum, 1=Mean, 2=Max, 3=Min, 4=Prod
    AllReduce = 0xC0,

    /// All-gather: gather tensors from all ranks to all ranks.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg`
    AllGather = 0xC1,

    /// Broadcast: send tensor from src rank to all ranks.
    ///
    /// Format: `dst:reg, tensor:reg, src:reg, group:reg`
    Broadcast = 0xC2,

    /// Reduce-scatter: reduce then scatter result.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg, op:u8`
    ReduceScatter = 0xC3,

    /// Barrier: synchronize all ranks.
    ///
    /// Format: `group:reg`
    Barrier = 0xC4,

    /// Pmap parallel sum collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPsum = 0xC5,

    /// Pmap parallel mean collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPmean = 0xC6,

    /// Pmap parallel max collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPmax = 0xC7,

    /// Pmap all-gather collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapAllGather = 0xC8,

    /// Vmap transformation.
    ///
    /// Format: `dst:reg, func:reg, in_axes:reg, out_axes:reg`
    VmapTransform = 0xC9,

    /// Pmap transformation.
    ///
    /// Format: `dst:reg, func:reg, axis_name:reg, in_axes:reg, out_axes:reg`
    PmapTransform = 0xCA,

    // ========================================================================
    // Process Group Operations (0xCB-0xCD)
    // ========================================================================

    /// Get the world process group (all ranks).
    ///
    /// Format: `dst:reg`
    /// Returns a handle to the default process group containing all ranks.
    DistWorldGroup = 0xCB,

    /// Create a new process group from a subset of ranks.
    ///
    /// Format: `dst:reg, ranks:reg`
    /// ranks: List of rank IDs to include in the new group.
    DistNewGroup = 0xCC,

    /// Get the rank of the current process in a group.
    ///
    /// Format: `dst:reg, group:reg`
    /// Returns the local rank ID within the specified group.
    DistGetRank = 0xCD,

    // ========================================================================
    // Point-to-Point Operations (0xCE-0xCF)
    // ========================================================================

    /// Send tensor to a specific rank.
    ///
    /// Format: `tensor:reg, dst_rank:reg, group:reg`
    /// Blocking send operation.
    P2PSend = 0xCE,

    /// Receive tensor from a specific rank.
    ///
    /// Format: `dst:reg, src_rank:reg, group:reg`
    /// Blocking receive operation.
    P2PRecv = 0xCF,

    // ========================================================================
    // Additional Collective Operations (0xD0-0xD1)
    // ========================================================================

    /// Collective gather: collect tensors from all ranks to one rank.
    ///
    /// Format: `dst:reg, tensor:reg, dst_rank:reg, group:reg`
    /// Only dst_rank receives the gathered result.
    CollectiveGather = 0xD0,

    /// Collective scatter: distribute tensor chunks from one rank to all ranks.
    ///
    /// Format: `dst:reg, tensor:reg, src_rank:reg, group:reg`
    /// Only src_rank provides the tensor to scatter.
    CollectiveScatter = 0xD1,

    // ========================================================================
    // Gradient Operations (0xD2-0xD5)
    // ========================================================================

    /// Bucket gradients for communication efficiency.
    ///
    /// Format: `dst:reg, gradients:reg, bucket_size:reg`
    /// Groups small gradients into larger communication buckets.
    BucketGradients = 0xD2,

    /// Get gradient from a parameter.
    ///
    /// Format: `dst:reg, param:reg`
    /// Returns the accumulated gradient for the parameter.
    GetGrad = 0xD3,

    /// Set gradient on a parameter.
    ///
    /// Format: `param:reg, grad:reg`
    /// Sets or accumulates gradient on the parameter.
    SetGrad = 0xD4,

    /// Execute backward pass on a module.
    ///
    /// Format: `dst:reg, module:reg, grad_output:reg`
    /// Returns gradients for module inputs.
    ModuleBackward = 0xD5,

    // ========================================================================
    // Actor Mesh Operations (0xD6-0xD7)
    // ========================================================================

    /// Select actors from a mesh by coordinates.
    ///
    /// Format: `dst:reg, mesh:reg, coords:reg`
    /// Returns a submesh or actor set from the mesh.
    MeshSelect = 0xD6,

    /// Create a new actor ID.
    ///
    /// Format: `dst:reg`
    /// Generates a unique actor identifier.
    ActorNewId = 0xD7,

    // ========================================================================
    // RDMA Operations (0xD8-0xDB)
    // ========================================================================

    /// Create an RDMA reference to a tensor.
    ///
    /// Format: `dst:reg, tensor:reg`
    /// Returns a remote-accessible reference.
    RdmaCreateRef = 0xD8,

    /// Fetch tensor data via RDMA.
    ///
    /// Format: `dst:reg, rdma_ref:reg`
    /// Zero-copy read from remote memory.
    RdmaFetch = 0xD9,

    /// Write tensor data via RDMA.
    ///
    /// Format: `rdma_ref:reg, tensor:reg`
    /// Zero-copy write to remote memory.
    RdmaWrite = 0xDA,

    /// Check if RDMA reference is still valid.
    ///
    /// Format: `dst:reg, rdma_ref:reg`
    /// Returns true if the reference is valid and accessible.
    RdmaCheckValid = 0xDB,

    // ========================================================================
    // Shape Manipulation Operations (0xDC-0xDF)
    // ========================================================================

    /// Unsqueeze tensor (add dimension of size 1).
    ///
    /// Format: `dst:reg, src:reg, dim:i8`
    /// Adds a dimension of size 1 at the specified position.
    Unsqueeze = 0xDC,

    /// Set scalar value in tensor at index.
    ///
    /// Format: `dst:reg, src:reg, indices:reg, value:reg`
    SetScalar = 0xDD,

    /// Make tensor contiguous in memory.
    ///
    /// Format: `dst:reg, src:reg`
    Contiguous = 0xDE,

    /// Move tensor to specified device.
    ///
    /// Format: `dst:reg, src:reg, device:reg`
    ToDevice = 0xDF,

    // ========================================================================
    // Regex Operations (0xE0-0xE3)
    // ========================================================================

    /// Find all matches of a regex pattern in text.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg`
    /// Returns a list of match strings.
    RegexFindAll = 0xE0,

    /// Replace all matches of a regex pattern in text.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg, replacement:reg`
    /// Returns the text with all matches replaced.
    RegexReplaceAll = 0xE1,

    /// Check if a regex pattern matches text.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg`
    /// Returns true if the pattern matches anywhere in text.
    RegexIsMatch = 0xE2,

    /// Split text by a regex pattern.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg`
    /// Returns a list of parts.
    RegexSplit = 0xE3,

    // ========================================================================
    // Tensor Creation & Utility Operations (0xE4-0xFF)
    // These operations provide tensor factory methods and utilities.
    // ========================================================================

    /// Create tensor with evenly spaced values (arange).
    ///
    /// Format: `dst:reg, start:reg, end:reg, step:reg, dtype:u8`
    Arange = 0xE4,

    /// Create tensor with evenly spaced values (linspace).
    ///
    /// Format: `dst:reg, start:reg, end:reg, steps:reg, dtype:u8`
    Linspace = 0xE5,

    /// Create random tensor.
    ///
    /// Format: `dst:reg, shape:reg, dtype:u8`
    Rand = 0xE6,

    /// Clone tensor.
    ///
    /// Format: `dst:reg, src:reg`
    Clone = 0xE7,

    /// Create identity matrix.
    ///
    /// Format: `dst:reg, size:reg, dtype:u8`
    Identity = 0xE8,

    /// Index tensor with indices.
    ///
    /// Format: `dst:reg, src:reg, indices:reg`
    Index = 0xE9,

    /// Concatenate tensors along axis.
    ///
    /// Format: `dst:reg, tensors:reg, axis:i8`
    Concat = 0xEA,

    /// Stack tensors along new axis.
    ///
    /// Format: `dst:reg, tensors:reg, axis:i8`
    Stack = 0xEB,

    /// Broadcast tensor to shape.
    ///
    /// Format: `dst:reg, src:reg, shape:reg`
    BroadcastToShape = 0xEC,

    /// Squeeze tensor dimensions.
    ///
    /// Format: `dst:reg, src:reg, axis:i8`
    Squeeze = 0xED,

    /// Compare tensors element-wise.
    ///
    /// Format: `dst:reg, a:reg, b:reg, op:u8`
    Cmp = 0xEE,

    /// Conditional select (where).
    ///
    /// Format: `dst:reg, cond:reg, a:reg, b:reg`
    Where = 0xEF,

    /// Clamp tensor values.
    ///
    /// Format: `dst:reg, src:reg, min:reg, max:reg`
    Clamp = 0xF0,

    /// Cast tensor to dtype.
    ///
    /// Format: `dst:reg, src:reg, dtype:u8`
    Cast = 0xF1,

    /// Masked fill.
    ///
    /// Format: `dst:reg, src:reg, mask:reg, value:reg`
    MaskedFill = 0xF2,

    /// Linear interpolation.
    ///
    /// Format: `dst:reg, a:reg, b:reg, t:reg`
    Lerp = 0xF3,

    /// Dot product.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    Dot = 0xF4,

    /// Convolution.
    ///
    /// Format: `dst:reg, input:reg, weight:reg, stride:reg, padding:reg`
    Conv = 0xF5,

    /// Batch matrix multiplication.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    BatchMatmul = 0xF6,

    /// Einsum.
    ///
    /// Format: `dst:reg, equation:reg, operands:reg`
    Einsum = 0xF7,

    /// Outer product.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    Outer = 0xF8,

    /// Cholesky decomposition.
    ///
    /// Format: `dst:reg, src:reg, upper:bool`
    Cholesky = 0xF9,

    /// Argmax along axis.
    ///
    /// Format: `dst:reg, src:reg, axis:i8, keepdim:bool`
    Argmax = 0xFA,

    /// Top-k elements.
    ///
    /// Format: `values:reg, indices:reg, src:reg, k:reg, dim:i8`
    Topk = 0xFB,

    /// Cumulative operation (sum, prod, max, min).
    ///
    /// Format: `dst:reg, src:reg, op:u8, axis:i8`
    Cumulative = 0xFC,

    /// Softmax.
    ///
    /// Format: `dst:reg, src:reg, axis:i8`
    Softmax = 0xFD,

    /// Layer normalization.
    ///
    /// Format: `dst:reg, src:reg, normalized_shape:reg, weight:reg, bias:reg, eps:f64`
    LayerNorm = 0xFE,

    /// Batch normalization.
    ///
    /// Format: `dst:reg, src:reg, weight:reg, bias:reg, running_mean:reg, running_var:reg`
    BatchNorm = 0xFF,
}

/// Additional tensor sub-opcodes that overflow the u8 range.
/// These use a two-byte encoding: [0xFC] [0x00] [ext_opcode:u8] [operands...]
///
/// This enum provides extended tensor operations beyond the 256-opcode limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorExtSubOpcode {
    /// RMS normalization.
    ///
    /// Format: `dst:reg, src:reg, weight:reg, eps:f64`
    RmsNorm = 0x00,

    /// Flash attention.
    ///
    /// Format: `dst:reg, q:reg, k:reg, v:reg, mask:reg, scale:f64`
    FlashAttention = 0x01,

    /// FFT (Fast Fourier Transform).
    ///
    /// Format: `dst:reg, src:reg, dim:i8`
    Fft = 0x02,

    /// Scatter operation.
    ///
    /// Format: `dst:reg, src:reg, index:reg, dim:i8`
    Scatter = 0x03,

    /// Create a contiguous view of tensor without copying.
    ///
    /// Format: `dst:reg, src:reg`
    ContiguousView = 0x04,

    /// Random unsigned 64-bit integer.
    ///
    /// Format: `dst:reg`
    RandomU64 = 0x05,

    /// Random float in custom range.
    ///
    /// Format: `dst:reg, low:reg, high:reg`
    RandomFloat = 0x06,

    /// Global allocator reference.
    ///
    /// Format: `dst:reg`
    GlobalAllocator = 0x07,

    /// Memory new ID allocation.
    ///
    /// Format: `dst:reg`
    MemNewId = 0x08,

    /// Memory allocate tensor.
    ///
    /// Format: `dst:reg, shape:reg, dtype:u8`
    MemAllocTensor = 0x09,

    // ========================================================================
    // Regex Single-Match / Capture Operations (0x0A-0x0C)
    // The four bulk regex ops (find_all, replace_all, is_match, split) live
    // in `TensorSubOpcode` 0xE0-0xE3; that range filled before the single-
    // match counterparts landed, so these three placed in the ext-extended
    // space (TensorExtended 0xFC + sub_op 0xFF + ext_sub_op).
    // ========================================================================

    /// Find the FIRST regex match in text.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg`
    /// Returns `Maybe<Text>` — the first match, or None.
    RegexFind = 0x0A,

    /// Replace the FIRST regex match in text.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg, replacement:reg`
    /// Returns the text with at most one match replaced.
    RegexReplace = 0x0B,

    /// Run a capturing regex and return ordered group captures of
    /// the first match.
    ///
    /// Format: `dst:reg, pattern:reg, text:reg`
    /// Returns `Maybe<List<Text>>` — the capture-group list (group
    /// 0 = whole match), or None if no match. Non-participating
    /// groups are emitted as empty strings; callers needing
    /// `Maybe<Text>` per group can re-check via the regex API.
    RegexCaptures = 0x0C,

    /// Wire-level permission check (#12 / P3.2).
    ///
    /// Format: `dst:reg, scope_tag:reg, target_id:reg`
    /// Routes a (scope_tag: u32, target_id: u64) pair through the
    /// runtime `PermissionRouter` and writes the decision tag
    /// into `dst` (0 = Allow, 1 = Deny). The Rust-side router
    /// holds the warm-path cache so repeats hit ≤2ns regardless
    /// of caller. NOT itself permission-gated — gating the
    /// gating intrinsic would create an infinite recursion in
    /// the dispatch path.
    ///
    /// Byte chosen at 0x1C — outside both `TensorSubOpcode`
    /// (0x00, 0x0D-0x1B, 0x20-…) and the regex window
    /// (0x0A-0x0C) so the decoder's TensorSubOpcode probe
    /// returns `None` and falls through to the extended
    /// dispatch path.
    PermissionCheckWire = 0x1C,

    /// Atomic permission assert (#12 / P3.2).
    ///
    /// Format: `scope_tag:u8, target_id:reg`
    /// Routes the check through `PermissionRouter`; on Allow
    /// proceeds to the next instruction with no observable
    /// effect, on Deny raises an interpreter `PermissionDenied`
    /// runtime error that surfaces to the catch frame as a
    /// typed Verum exception.
    ///
    /// The single-instruction shape lets the codegen emit a
    /// gate prologue without branching machinery — the dispatch
    /// handler holds all the deny-path logic.  Designed to be
    /// auto-emitted by the AST→VBC pass before any intrinsic
    /// carrying `IntrinsicHint::RequiresPermission`.
    ///
    /// Byte 0x1D, picked one past `PermissionCheckWire` for
    /// locality.
    PermissionAssert = 0x1D,

    /// Read a single field of the runtime
    /// `PermissionRouterStats` struct (#101).
    ///
    /// Format: `dst:reg, selector:reg`
    /// Selector encoding (matches the field order in
    /// `verum_vbc::interpreter::permission::PermissionRouterStats`):
    ///   * 0 → total
    ///   * 1 → last_entry_hits
    ///   * 2 → map_hits
    ///   * 3 → policy_invocations
    ///   * 4 → denials
    ///
    /// Out-of-range selectors return 0 — the dispatch handler
    /// treats unknown selectors as "no such stat" rather than
    /// raising, so version-skew between callers and runtime
    /// fails open instead of crashing.  Stdlib's typed
    /// `permission_stats()` wrapper sources its 5 fields by
    /// calling this intrinsic five times.
    PermissionStatsRead = 0x1E,

    /// Clear the runtime `PermissionRouter` stats (#101).
    ///
    /// Format: `dst:reg`
    /// Resets total / hits / misses / denials to zero;
    /// preserves the cache itself (use the dedicated
    /// `clear_permission_cache` API for that).  The `dst`
    /// receives Unit so the opcode round-trips through the
    /// register-allocator like every other intrinsic call.
    PermissionStatsClear = 0x1F,
}

impl TensorExtSubOpcode {
    /// Creates a tensor ext sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::RmsNorm),
            0x01 => Some(Self::FlashAttention),
            0x02 => Some(Self::Fft),
            0x03 => Some(Self::Scatter),
            0x04 => Some(Self::ContiguousView),
            0x05 => Some(Self::RandomU64),
            0x06 => Some(Self::RandomFloat),
            0x07 => Some(Self::GlobalAllocator),
            0x08 => Some(Self::MemNewId),
            0x09 => Some(Self::MemAllocTensor),
            0x0A => Some(Self::RegexFind),
            0x0B => Some(Self::RegexReplace),
            0x0C => Some(Self::RegexCaptures),
            0x1C => Some(Self::PermissionCheckWire),
            0x1D => Some(Self::PermissionAssert),
            0x1E => Some(Self::PermissionStatsRead),
            0x1F => Some(Self::PermissionStatsClear),
            _ => None,
        }
    }

    /// Returns the byte value of this tensor ext sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this tensor ext sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::RmsNorm => "TENSOR_RMS_NORM",
            Self::FlashAttention => "TENSOR_FLASH_ATTENTION",
            Self::Fft => "TENSOR_FFT",
            Self::Scatter => "TENSOR_SCATTER",
            Self::ContiguousView => "TENSOR_CONTIGUOUS_VIEW",
            Self::RandomU64 => "RANDOM_U64",
            Self::RandomFloat => "RANDOM_FLOAT",
            Self::GlobalAllocator => "GLOBAL_ALLOCATOR",
            Self::MemNewId => "MEM_NEW_ID",
            Self::MemAllocTensor => "MEM_ALLOC_TENSOR",
            Self::RegexFind => "REGEX_FIND",
            Self::RegexReplace => "REGEX_REPLACE",
            Self::RegexCaptures => "REGEX_CAPTURES",
            Self::PermissionCheckWire => "PERMISSION_CHECK_WIRE",
            Self::PermissionAssert => "PERMISSION_ASSERT",
            Self::PermissionStatsRead => "PERMISSION_STATS_READ",
            Self::PermissionStatsClear => "PERMISSION_STATS_CLEAR",
        }
    }
}

impl TensorSubOpcode {
    /// Creates a tensor sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Pooling
            0x00 => Some(Self::Pool),
            // Register-based Tensor Operations
            0x0D => Some(Self::NewFromArgs),
            0x0E => Some(Self::FillFromArgs),
            0x0F => Some(Self::FromSliceArgs),
            // Reduction Variants
            0x10 => Some(Self::Argmin),
            0x11 => Some(Self::Nansum),
            0x12 => Some(Self::Nanmean),
            // Register-based Tensor Operations (cont.)
            0x13 => Some(Self::BinopFromArgs),
            0x14 => Some(Self::UnopFromArgs),
            0x15 => Some(Self::MatmulFromArgs),
            0x16 => Some(Self::ReduceFromArgs),
            0x17 => Some(Self::ReshapeFromArgs),
            0x18 => Some(Self::TransposeFromArgs),
            0x19 => Some(Self::SliceFromArgs),
            0x1A => Some(Self::GetElementFromArgs),
            0x1B => Some(Self::SetElementFromArgs),
            // Advanced Indexing
            0x20 => Some(Self::Gather),
            0x21 => Some(Self::Permute),
            0x22 => Some(Self::Flip),
            0x23 => Some(Self::Roll),
            // Linear System Solvers
            0x30 => Some(Self::Solve),
            0x31 => Some(Self::Lstsq),
            0x32 => Some(Self::TriSolve),
            // Matrix Decompositions
            0x40 => Some(Self::QR),
            0x41 => Some(Self::SVD),
            0x42 => Some(Self::LU),
            0x43 => Some(Self::Eig),
            0x44 => Some(Self::EigSymmetric),
            0x45 => Some(Self::Schur),
            // Matrix Properties
            0x60 => Some(Self::Det),
            0x61 => Some(Self::Rank),
            0x62 => Some(Self::Cond),
            0x63 => Some(Self::Trace),
            0x64 => Some(Self::Norm),
            // Advanced Operations
            0x70 => Some(Self::Kron),
            0x71 => Some(Self::Cross),
            0x72 => Some(Self::Contract),
            0x73 => Some(Self::MatrixPower),
            0x74 => Some(Self::Expm),
            0x75 => Some(Self::Logm),
            0x76 => Some(Self::Inverse),
            0x77 => Some(Self::Rfft),
            0x78 => Some(Self::Irfft),
            0x79 => Some(Self::ComplexMul),
            0x7A => Some(Self::ComplexPow),
            0x7B => Some(Self::SsmScan),
            0x7C => Some(Self::Uniform),
            0x7D => Some(Self::Bincount),
            0x7E => Some(Self::GatherNd),
            0x7F => Some(Self::ArangeUsize),
            // Extended Tensor Operations
            0x80 => Some(Self::Repeat),
            0x81 => Some(Self::Tanh),
            0x82 => Some(Self::SumAll),
            0x83 => Some(Self::FromArray),
            0x84 => Some(Self::IsTraining),
            0x85 => Some(Self::RandomFloat01),
            0x86 => Some(Self::MaskedSelect),
            0x87 => Some(Self::LeakyRelu),
            0x88 => Some(Self::Diag),
            0x89 => Some(Self::Triu),
            0x8A => Some(Self::Tril),
            0x8B => Some(Self::Nonzero),
            0x8C => Some(Self::OneHot),
            0x8D => Some(Self::Split),
            0x8E => Some(Self::SplitAt),
            0x8F => Some(Self::GetScalar),
            // Tokenizer Operations
            0x90 => Some(Self::TokenizerLoadBpe),
            0x91 => Some(Self::TokenizerLoadPretrained),
            0x92 => Some(Self::TokenizerEncode),
            0x93 => Some(Self::TokenizerDecode),
            0x94 => Some(Self::TokenizerLoadSpm),
            0x95 => Some(Self::TokenizerSpmEncode),
            0x96 => Some(Self::TokenizerSpmDecode),
            // Sampling Operations
            0xA0 => Some(Self::SampleTopP),
            0xA1 => Some(Self::SampleTemperature),
            0xA2 => Some(Self::PagedAttention),
            // Inference Utility Operations
            0xB0 => Some(Self::ParseToolCall),
            0xB1 => Some(Self::FormatValue),
            0xB2 => Some(Self::TensorFromSliceUsize),
            0xB3 => Some(Self::QuantizedMatmul),
            0xB4 => Some(Self::TensorNorm),
            0xB5 => Some(Self::GenerateRequestId),
            0xB6 => Some(Self::JsonSchemaToJson),
            0xB7 => Some(Self::FunctionSchemaToJson),
            0xB8 => Some(Self::ParseFunctionCalls),
            // Distributed/Collective Operations
            0xC0 => Some(Self::AllReduce),
            0xC1 => Some(Self::AllGather),
            0xC2 => Some(Self::Broadcast),
            0xC3 => Some(Self::ReduceScatter),
            0xC4 => Some(Self::Barrier),
            0xC5 => Some(Self::PmapPsum),
            0xC6 => Some(Self::PmapPmean),
            0xC7 => Some(Self::PmapPmax),
            0xC8 => Some(Self::PmapAllGather),
            0xC9 => Some(Self::VmapTransform),
            0xCA => Some(Self::PmapTransform),
            // Process Group Operations
            0xCB => Some(Self::DistWorldGroup),
            0xCC => Some(Self::DistNewGroup),
            0xCD => Some(Self::DistGetRank),
            // Point-to-Point Operations
            0xCE => Some(Self::P2PSend),
            0xCF => Some(Self::P2PRecv),
            // Additional Collective Operations
            0xD0 => Some(Self::CollectiveGather),
            0xD1 => Some(Self::CollectiveScatter),
            // Gradient Operations
            0xD2 => Some(Self::BucketGradients),
            0xD3 => Some(Self::GetGrad),
            0xD4 => Some(Self::SetGrad),
            0xD5 => Some(Self::ModuleBackward),
            // Actor Mesh Operations
            0xD6 => Some(Self::MeshSelect),
            0xD7 => Some(Self::ActorNewId),
            // RDMA Operations
            0xD8 => Some(Self::RdmaCreateRef),
            0xD9 => Some(Self::RdmaFetch),
            0xDA => Some(Self::RdmaWrite),
            0xDB => Some(Self::RdmaCheckValid),
            // Shape Manipulation Operations
            0xDC => Some(Self::Unsqueeze),
            0xDD => Some(Self::SetScalar),
            0xDE => Some(Self::Contiguous),
            0xDF => Some(Self::ToDevice),
            // Regex Operations
            0xE0 => Some(Self::RegexFindAll),
            0xE1 => Some(Self::RegexReplaceAll),
            0xE2 => Some(Self::RegexIsMatch),
            0xE3 => Some(Self::RegexSplit),
            // Tensor Creation & Utility Operations
            0xE4 => Some(Self::Arange),
            0xE5 => Some(Self::Linspace),
            0xE6 => Some(Self::Rand),
            0xE7 => Some(Self::Clone),
            0xE8 => Some(Self::Identity),
            0xE9 => Some(Self::Index),
            0xEA => Some(Self::Concat),
            0xEB => Some(Self::Stack),
            0xEC => Some(Self::BroadcastToShape),
            0xED => Some(Self::Squeeze),
            0xEE => Some(Self::Cmp),
            0xEF => Some(Self::Where),
            0xF0 => Some(Self::Clamp),
            0xF1 => Some(Self::Cast),
            0xF2 => Some(Self::MaskedFill),
            0xF3 => Some(Self::Lerp),
            0xF4 => Some(Self::Dot),
            0xF5 => Some(Self::Conv),
            0xF6 => Some(Self::BatchMatmul),
            0xF7 => Some(Self::Einsum),
            0xF8 => Some(Self::Outer),
            0xF9 => Some(Self::Cholesky),
            0xFA => Some(Self::Argmax),
            0xFB => Some(Self::Topk),
            0xFC => Some(Self::Cumulative),
            0xFD => Some(Self::Softmax),
            0xFE => Some(Self::LayerNorm),
            0xFF => Some(Self::BatchNorm),
            _ => None,
        }
    }

    /// Returns the byte value of this tensor sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this tensor sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Pooling
            Self::Pool => "TENSOR_POOL",
            // Register-based ops
            Self::NewFromArgs => "TENSOR_NEW_ARGS",
            Self::FillFromArgs => "TENSOR_FILL_ARGS",
            Self::FromSliceArgs => "TENSOR_FROM_SLICE_ARGS",
            Self::BinopFromArgs => "TENSOR_BINOP_ARGS",
            Self::UnopFromArgs => "TENSOR_UNOP_ARGS",
            Self::MatmulFromArgs => "TENSOR_MATMUL_ARGS",
            Self::ReduceFromArgs => "TENSOR_REDUCE_ARGS",
            Self::ReshapeFromArgs => "TENSOR_RESHAPE_ARGS",
            Self::TransposeFromArgs => "TENSOR_TRANSPOSE_ARGS",
            Self::SliceFromArgs => "TENSOR_SLICE_ARGS",
            Self::GetElementFromArgs => "TENSOR_GET_ELEMENT_ARGS",
            Self::SetElementFromArgs => "TENSOR_SET_ELEMENT_ARGS",
            // Reduction Variants
            Self::Argmin => "TENSOR_ARGMIN_EXT",
            Self::Nansum => "TENSOR_NANSUM",
            Self::Nanmean => "TENSOR_NANMEAN",
            // Advanced Indexing
            Self::Gather => "TENSOR_GATHER",
            Self::Permute => "TENSOR_PERMUTE",
            Self::Flip => "TENSOR_FLIP",
            Self::Roll => "TENSOR_ROLL",
            // Linear System Solvers
            Self::Solve => "TENSOR_SOLVE",
            Self::Lstsq => "TENSOR_LSTSQ",
            Self::TriSolve => "TENSOR_TRI_SOLVE_EXT",
            // Matrix Decompositions
            Self::QR => "TENSOR_QR",
            Self::SVD => "TENSOR_SVD",
            Self::LU => "TENSOR_LU",
            Self::Eig => "TENSOR_EIG",
            Self::EigSymmetric => "TENSOR_EIG_SYM",
            Self::Schur => "TENSOR_SCHUR",
            // Matrix Properties
            Self::Det => "TENSOR_DET",
            Self::Rank => "TENSOR_RANK",
            Self::Cond => "TENSOR_COND",
            Self::Trace => "TENSOR_TRACE",
            Self::Norm => "TENSOR_NORM",
            // Advanced Operations
            Self::Kron => "TENSOR_KRON",
            Self::Cross => "TENSOR_CROSS",
            Self::Contract => "TENSOR_CONTRACT",
            Self::MatrixPower => "TENSOR_MATRIX_POW",
            Self::Expm => "TENSOR_EXPM",
            Self::Logm => "TENSOR_LOGM",
            Self::Inverse => "TENSOR_INVERSE",
            Self::Rfft => "TENSOR_RFFT",
            Self::Irfft => "TENSOR_IRFFT",
            Self::ComplexMul => "TENSOR_COMPLEX_MUL",
            Self::ComplexPow => "TENSOR_COMPLEX_POW",
            Self::SsmScan => "TENSOR_SSM_SCAN",
            Self::Uniform => "TENSOR_UNIFORM",
            Self::Bincount => "TENSOR_BINCOUNT",
            Self::GatherNd => "TENSOR_GATHER_ND",
            Self::ArangeUsize => "TENSOR_ARANGE_USIZE",
            // Extended Tensor Operations
            Self::Repeat => "TENSOR_REPEAT",
            Self::Tanh => "TENSOR_TANH",
            Self::SumAll => "TENSOR_SUM_ALL",
            Self::FromArray => "TENSOR_FROM_ARRAY",
            Self::IsTraining => "TENSOR_IS_TRAINING",
            Self::RandomFloat01 => "TENSOR_RANDOM_FLOAT_01",
            Self::MaskedSelect => "TENSOR_MASKED_SELECT",
            Self::LeakyRelu => "TENSOR_LEAKY_RELU",
            Self::Diag => "TENSOR_DIAG",
            Self::Triu => "TENSOR_TRIU",
            Self::Tril => "TENSOR_TRIL",
            Self::Nonzero => "TENSOR_NONZERO",
            Self::OneHot => "TENSOR_ONE_HOT",
            Self::Split => "TENSOR_SPLIT",
            Self::SplitAt => "TENSOR_SPLIT_AT",
            Self::GetScalar => "TENSOR_GET_SCALAR",
            // Tokenizer Operations
            Self::TokenizerLoadBpe => "TOKENIZER_LOAD_BPE",
            Self::TokenizerLoadPretrained => "TOKENIZER_LOAD_PRETRAINED",
            Self::TokenizerEncode => "TOKENIZER_ENCODE",
            Self::TokenizerDecode => "TOKENIZER_DECODE",
            Self::TokenizerLoadSpm => "TOKENIZER_LOAD_SPM",
            Self::TokenizerSpmEncode => "TOKENIZER_SPM_ENCODE",
            Self::TokenizerSpmDecode => "TOKENIZER_SPM_DECODE",
            // Sampling Operations
            Self::SampleTopP => "SAMPLE_TOP_P",
            Self::SampleTemperature => "SAMPLE_TEMPERATURE",
            Self::PagedAttention => "PAGED_ATTENTION",
            // Inference Utility Operations
            Self::ParseToolCall => "PARSE_TOOL_CALL",
            Self::FormatValue => "FORMAT_VALUE",
            Self::TensorFromSliceUsize => "TENSOR_FROM_SLICE_USIZE",
            Self::QuantizedMatmul => "QUANTIZED_MATMUL",
            Self::TensorNorm => "TENSOR_NORM",
            Self::GenerateRequestId => "GENERATE_REQUEST_ID",
            Self::JsonSchemaToJson => "JSON_SCHEMA_TO_JSON",
            Self::FunctionSchemaToJson => "FUNCTION_SCHEMA_TO_JSON",
            Self::ParseFunctionCalls => "PARSE_FUNCTION_CALLS",
            // Distributed/Collective Operations
            Self::AllReduce => "COLLECTIVE_ALL_REDUCE",
            Self::AllGather => "COLLECTIVE_ALL_GATHER",
            Self::Broadcast => "COLLECTIVE_BROADCAST",
            Self::ReduceScatter => "COLLECTIVE_REDUCE_SCATTER",
            Self::Barrier => "COLLECTIVE_BARRIER",
            Self::PmapPsum => "PMAP_PSUM",
            Self::PmapPmean => "PMAP_PMEAN",
            Self::PmapPmax => "PMAP_PMAX",
            Self::PmapAllGather => "PMAP_ALL_GATHER",
            Self::VmapTransform => "VMAP_TRANSFORM",
            Self::PmapTransform => "PMAP_TRANSFORM",
            // Process Group Operations
            Self::DistWorldGroup => "DIST_WORLD_GROUP",
            Self::DistNewGroup => "DIST_NEW_GROUP",
            Self::DistGetRank => "DIST_GET_RANK",
            // Point-to-Point Operations
            Self::P2PSend => "P2P_SEND",
            Self::P2PRecv => "P2P_RECV",
            // Additional Collective Operations
            Self::CollectiveGather => "COLLECTIVE_GATHER",
            Self::CollectiveScatter => "COLLECTIVE_SCATTER",
            // Gradient Operations
            Self::BucketGradients => "BUCKET_GRADIENTS",
            Self::GetGrad => "GET_GRAD",
            Self::SetGrad => "SET_GRAD",
            Self::ModuleBackward => "MODULE_BACKWARD",
            // Actor Mesh Operations
            Self::MeshSelect => "MESH_SELECT",
            Self::ActorNewId => "ACTOR_NEW_ID",
            // RDMA Operations
            Self::RdmaCreateRef => "RDMA_CREATE_REF",
            Self::RdmaFetch => "RDMA_FETCH",
            Self::RdmaWrite => "RDMA_WRITE",
            Self::RdmaCheckValid => "RDMA_CHECK_VALID",
            // Shape Manipulation Operations
            Self::Unsqueeze => "TENSOR_UNSQUEEZE",
            Self::SetScalar => "TENSOR_SET_SCALAR",
            Self::Contiguous => "TENSOR_CONTIGUOUS",
            Self::ToDevice => "TENSOR_TO_DEVICE",
            // Regex Operations
            Self::RegexFindAll => "REGEX_FIND_ALL",
            Self::RegexReplaceAll => "REGEX_REPLACE_ALL",
            Self::RegexIsMatch => "REGEX_IS_MATCH",
            Self::RegexSplit => "REGEX_SPLIT",
            // Tensor Creation & Utility Operations
            Self::Arange => "TENSOR_ARANGE",
            Self::Linspace => "TENSOR_LINSPACE",
            Self::Rand => "TENSOR_RAND",
            Self::Clone => "TENSOR_CLONE",
            Self::Identity => "TENSOR_IDENTITY",
            Self::Index => "TENSOR_INDEX",
            Self::Concat => "TENSOR_CONCAT",
            Self::Stack => "TENSOR_STACK",
            Self::BroadcastToShape => "TENSOR_BROADCAST_TO_SHAPE",
            Self::Squeeze => "TENSOR_SQUEEZE",
            Self::Cmp => "TENSOR_CMP",
            Self::Where => "TENSOR_WHERE",
            Self::Clamp => "TENSOR_CLAMP",
            Self::Cast => "TENSOR_CAST",
            Self::MaskedFill => "TENSOR_MASKED_FILL",
            Self::Lerp => "TENSOR_LERP",
            Self::Dot => "TENSOR_DOT",
            Self::Conv => "TENSOR_CONV",
            Self::BatchMatmul => "TENSOR_BATCH_MATMUL",
            Self::Einsum => "TENSOR_EINSUM",
            Self::Outer => "TENSOR_OUTER",
            Self::Cholesky => "TENSOR_CHOLESKY",
            Self::Argmax => "TENSOR_ARGMAX",
            Self::Topk => "TENSOR_TOPK",
            Self::Cumulative => "TENSOR_CUMULATIVE",
            Self::Softmax => "TENSOR_SOFTMAX",
            Self::LayerNorm => "TENSOR_LAYER_NORM",
            Self::BatchNorm => "TENSOR_BATCH_NORM",
        }
    }

    /// Returns the category of this tensor sub-opcode.
    pub fn category(self) -> &'static str {
        match self as u8 {
            0x00..=0x0F => "Pooling",
            0x10..=0x1F => "Reduction Variants",
            0x20..=0x2F => "Advanced Indexing",
            0x30..=0x3F => "Linear System Solvers",
            0x40..=0x5F => "Matrix Decompositions",
            0x60..=0x6F => "Matrix Properties",
            0x70..=0x7F => "Advanced Operations",
            0x80..=0x8F => "Extended Tensor Operations",
            0x90..=0x9F => "Tokenizer Operations",
            0xA0..=0xAF => "Sampling Operations",
            0xB0..=0xBF => "Inference Utility Operations",
            0xC0..=0xCF => "Distributed/Collective Operations",
            0xD0..=0xDF => "Gradient/RDMA Operations",
            0xE0..=0xE3 => "Regex Operations",
            0xE4..=0xFF => "Tensor Creation & Utility Operations",
        }
    }

    /// Returns true if this operation produces multiple outputs.
    pub fn has_multiple_outputs(self) -> bool {
        matches!(
            self,
            Self::QR
                | Self::SVD
                | Self::LU
                | Self::Eig
                | Self::EigSymmetric
                | Self::Schur
                | Self::Lstsq
        )
    }

    /// Returns true if this operation requires square input.
    pub fn requires_square(self) -> bool {
        matches!(
            self,
            Self::Det
                | Self::Eig
                | Self::EigSymmetric
                | Self::Schur
                | Self::MatrixPower
                | Self::Expm
                | Self::Logm
        )
    }
}

// ============================================================================
// ML Extended Sub-Opcodes
// ============================================================================

/// ML extended sub-opcodes for use with `MlExtended` (0xFD) prefix.
///
/// Provides specialized ML/AI operations separated from tensor ops:
/// - Tokenizer operations for text processing
/// - Sampling operations for inference
/// - Inference utilities for LLM serving
/// - Distributed training operations
///
/// # Encoding
///
/// ```text
/// [0xFD] [sub_opcode:u8] [operands...]
/// ```
///
/// # Example
///
/// ```text
/// // Encode text to tokens
/// MlExtended TokenizerEncode dst:r0, tokenizer:r1, text:r2
///
/// // Top-p sampling
/// MlExtended SampleTopP dst:r0, logits:r1, p:r2
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MlSubOpcode {
    // ========================================================================
    // Tokenizer Operations (0x00-0x0F)
    // ========================================================================

    /// Load BPE tokenizer from files.
    ///
    /// Format: `dst:reg, vocab_path:reg, merges_path:reg`
    TokenizerLoadBpe = 0x00,

    /// Load pretrained tokenizer by model name.
    ///
    /// Format: `dst:reg, model_name:reg`
    TokenizerLoadPretrained = 0x01,

    /// Encode text to tokens.
    ///
    /// Format: `dst:reg, tokenizer:reg, text:reg`
    TokenizerEncode = 0x02,

    /// Decode tokens to text.
    ///
    /// Format: `dst:reg, tokenizer:reg, tokens:reg`
    TokenizerDecode = 0x03,

    /// Load SentencePiece model.
    ///
    /// Format: `dst:reg, model_path:reg`
    TokenizerLoadSpm = 0x04,

    /// Encode with SentencePiece.
    ///
    /// Format: `dst:reg, tokenizer:reg, text:reg`
    TokenizerSpmEncode = 0x05,

    /// Decode with SentencePiece.
    ///
    /// Format: `dst:reg, tokenizer:reg, tokens:reg`
    TokenizerSpmDecode = 0x06,

    // ========================================================================
    // Sampling Operations (0x10-0x1F)
    // ========================================================================

    /// Top-p (nucleus) sampling.
    ///
    /// Format: `dst:reg, logits:reg, p:reg`
    SampleTopP = 0x10,

    /// Temperature sampling.
    ///
    /// Format: `dst:reg, logits:reg, temperature:reg`
    SampleTemperature = 0x11,

    /// Paged attention for KV cache.
    ///
    /// Format: `dst:reg, q:reg, kv_cache:reg, block_table:reg, context_len:reg`
    PagedAttention = 0x12,

    /// Top-k sampling.
    ///
    /// Format: `dst:reg, logits:reg, k:reg`
    SampleTopK = 0x13,

    /// Combined top-k + top-p sampling.
    ///
    /// Format: `dst:reg, logits:reg, k:reg, p:reg`
    SampleTopKTopP = 0x14,

    /// Repetition penalty.
    ///
    /// Format: `dst:reg, logits:reg, past_tokens:reg, penalty:reg`
    RepetitionPenalty = 0x15,

    // ========================================================================
    // Inference Utility Operations (0x20-0x2F)
    // ========================================================================

    /// Parse tool call from action string.
    ///
    /// Format: `dst:reg, action:reg`
    ParseToolCall = 0x20,

    /// Format value for display.
    ///
    /// Format: `dst:reg, value:reg`
    FormatValue = 0x21,

    /// Quantized matrix multiplication.
    ///
    /// Format: `dst:reg, input:reg, weight:reg, scale:reg, zero_point:reg`
    QuantizedMatmul = 0x22,

    /// Generate unique request ID.
    ///
    /// Format: `dst:reg`
    GenerateRequestId = 0x23,

    /// Convert JSON schema to JSON.
    ///
    /// Format: `dst:reg, schema:reg`
    JsonSchemaToJson = 0x24,

    /// Convert function schema to JSON.
    ///
    /// Format: `dst:reg, schema:reg`
    FunctionSchemaToJson = 0x25,

    /// Parse function calls from response.
    ///
    /// Format: `dst:reg, response:reg`
    ParseFunctionCalls = 0x26,

    /// KV cache operations.
    ///
    /// Format: `dst:reg, op:u8, cache:reg, [operands...]`
    /// op: 0=create, 1=append, 2=truncate, 3=clear
    KvCacheOp = 0x27,

    /// Speculative decoding accept/reject.
    ///
    /// Format: `dst:reg, draft_tokens:reg, target_probs:reg`
    SpeculativeVerify = 0x28,

    // ========================================================================
    // Distributed/Collective Operations (0x30-0x3F)
    // ========================================================================

    /// All-reduce: reduce tensor across all ranks and distribute result.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg, op:u8`
    /// Op: 0=Sum, 1=Mean, 2=Max, 3=Min, 4=Prod
    AllReduce = 0x30,

    /// All-gather: gather tensors from all ranks to all ranks.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg`
    AllGather = 0x31,

    /// Broadcast: send tensor from src rank to all ranks.
    ///
    /// Format: `dst:reg, tensor:reg, src:reg, group:reg`
    Broadcast = 0x32,

    /// Reduce-scatter: reduce then scatter result.
    ///
    /// Format: `dst:reg, tensor:reg, group:reg, op:u8`
    ReduceScatter = 0x33,

    /// Barrier: synchronize all ranks.
    ///
    /// Format: `group:reg`
    Barrier = 0x34,

    /// Pmap parallel sum collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPsum = 0x35,

    /// Pmap parallel mean collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPmean = 0x36,

    /// Pmap parallel max collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapPmax = 0x37,

    /// Pmap all-gather collective.
    ///
    /// Format: `dst:reg, tensor:reg, axis_name:reg`
    PmapAllGather = 0x38,

    /// Vmap transformation.
    ///
    /// Format: `dst:reg, func:reg, in_axes:reg, out_axes:reg`
    VmapTransform = 0x39,

    /// Pmap transformation.
    ///
    /// Format: `dst:reg, func:reg, axis_name:reg, in_axes:reg, out_axes:reg`
    PmapTransform = 0x3A,

    // ========================================================================
    // Process Group Operations (0x40-0x4F)
    // ========================================================================

    /// Get the world process group (all ranks).
    ///
    /// Format: `dst:reg`
    DistWorldGroup = 0x40,

    /// Create a new process group from a subset of ranks.
    ///
    /// Format: `dst:reg, ranks:reg`
    DistNewGroup = 0x41,

    /// Get the rank of the current process in a group.
    ///
    /// Format: `dst:reg, group:reg`
    DistGetRank = 0x42,

    /// Get the world size (total number of ranks).
    ///
    /// Format: `dst:reg`
    DistWorldSize = 0x43,

    /// Get the local rank (within a node).
    ///
    /// Format: `dst:reg`
    DistLocalRank = 0x44,

    // ========================================================================
    // Point-to-Point Operations (0x50-0x5F)
    // ========================================================================

    /// Send tensor to a specific rank.
    ///
    /// Format: `tensor:reg, dst_rank:reg, group:reg`
    P2PSend = 0x50,

    /// Receive tensor from a specific rank.
    ///
    /// Format: `dst:reg, src_rank:reg, group:reg`
    P2PRecv = 0x51,

    /// Async send (returns handle).
    ///
    /// Format: `handle:reg, tensor:reg, dst_rank:reg, group:reg`
    P2PIsend = 0x52,

    /// Async receive (returns handle).
    ///
    /// Format: `handle:reg, dst:reg, src_rank:reg, group:reg`
    P2PIrecv = 0x53,

    /// Wait for async operation.
    ///
    /// Format: `handle:reg`
    P2PWait = 0x54,

    // ========================================================================
    // Gradient Operations (0x60-0x6F)
    // ========================================================================

    /// Bucket gradients for communication efficiency.
    ///
    /// Format: `dst:reg, gradients:reg, bucket_size:reg`
    BucketGradients = 0x60,

    /// Get gradient from a parameter.
    ///
    /// Format: `dst:reg, param:reg`
    GetGrad = 0x61,

    /// Set gradient on a parameter.
    ///
    /// Format: `param:reg, grad:reg`
    SetGrad = 0x62,

    /// Execute backward pass on a module.
    ///
    /// Format: `dst:reg, module:reg, grad_output:reg`
    ModuleBackward = 0x63,

    /// Zero gradients.
    ///
    /// Format: `params:reg`
    ZeroGrad = 0x64,

    /// Gradient clipping.
    ///
    /// Format: `params:reg, max_norm:reg`
    ClipGradNorm = 0x65,

    /// Begin forward-mode autodiff (JVP - Jacobian-Vector Product).
    ///
    /// Format: `dst:reg, primals:reg, tangents:reg`
    JvpBegin = 0x66,

    /// End forward-mode autodiff scope.
    ///
    /// Format: `dst:reg, scope:reg`
    JvpEnd = 0x67,

    /// Register custom gradient function.
    ///
    /// Format: `dst:reg, forward_fn:reg, backward_fn:reg`
    GradCustom = 0x68,

    /// Zero out tangent vectors (forward-mode specific).
    ///
    /// Format: `tangents:reg`
    GradZeroTangent = 0x69,

    /// Recompute forward pass during backward (activation checkpointing).
    ///
    /// Format: `dst:reg, checkpoint:reg, grad_output:reg`
    GradRecompute = 0x6A,

    // ========================================================================
    // Actor/Mesh Operations (0x70-0x7F)
    // ========================================================================

    /// Select actors from a mesh by coordinates.
    ///
    /// Format: `dst:reg, mesh:reg, coords:reg`
    MeshSelect = 0x70,

    /// Create a new actor ID.
    ///
    /// Format: `dst:reg`
    ActorNewId = 0x71,

    /// Create actor mesh.
    ///
    /// Format: `dst:reg, shape:reg`
    MeshCreate = 0x72,

    /// Get mesh shape.
    ///
    /// Format: `dst:reg, mesh:reg`
    MeshShape = 0x73,

    // ========================================================================
    // RDMA Operations (0x80-0x8F)
    // ========================================================================

    /// Create an RDMA reference to a tensor.
    ///
    /// Format: `dst:reg, tensor:reg`
    RdmaCreateRef = 0x80,

    /// Fetch tensor data via RDMA.
    ///
    /// Format: `dst:reg, rdma_ref:reg`
    RdmaFetch = 0x81,

    /// Write tensor data via RDMA.
    ///
    /// Format: `rdma_ref:reg, tensor:reg`
    RdmaWrite = 0x82,

    /// Check if RDMA reference is still valid.
    ///
    /// Format: `dst:reg, rdma_ref:reg`
    RdmaCheckValid = 0x83,
}

impl MlSubOpcode {
    /// Creates an ML sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Tokenizer Operations
            0x00 => Some(Self::TokenizerLoadBpe),
            0x01 => Some(Self::TokenizerLoadPretrained),
            0x02 => Some(Self::TokenizerEncode),
            0x03 => Some(Self::TokenizerDecode),
            0x04 => Some(Self::TokenizerLoadSpm),
            0x05 => Some(Self::TokenizerSpmEncode),
            0x06 => Some(Self::TokenizerSpmDecode),
            // Sampling Operations
            0x10 => Some(Self::SampleTopP),
            0x11 => Some(Self::SampleTemperature),
            0x12 => Some(Self::PagedAttention),
            0x13 => Some(Self::SampleTopK),
            0x14 => Some(Self::SampleTopKTopP),
            0x15 => Some(Self::RepetitionPenalty),
            // Inference Utility Operations
            0x20 => Some(Self::ParseToolCall),
            0x21 => Some(Self::FormatValue),
            0x22 => Some(Self::QuantizedMatmul),
            0x23 => Some(Self::GenerateRequestId),
            0x24 => Some(Self::JsonSchemaToJson),
            0x25 => Some(Self::FunctionSchemaToJson),
            0x26 => Some(Self::ParseFunctionCalls),
            0x27 => Some(Self::KvCacheOp),
            0x28 => Some(Self::SpeculativeVerify),
            // Distributed/Collective Operations
            0x30 => Some(Self::AllReduce),
            0x31 => Some(Self::AllGather),
            0x32 => Some(Self::Broadcast),
            0x33 => Some(Self::ReduceScatter),
            0x34 => Some(Self::Barrier),
            0x35 => Some(Self::PmapPsum),
            0x36 => Some(Self::PmapPmean),
            0x37 => Some(Self::PmapPmax),
            0x38 => Some(Self::PmapAllGather),
            0x39 => Some(Self::VmapTransform),
            0x3A => Some(Self::PmapTransform),
            // Process Group Operations
            0x40 => Some(Self::DistWorldGroup),
            0x41 => Some(Self::DistNewGroup),
            0x42 => Some(Self::DistGetRank),
            0x43 => Some(Self::DistWorldSize),
            0x44 => Some(Self::DistLocalRank),
            // Point-to-Point Operations
            0x50 => Some(Self::P2PSend),
            0x51 => Some(Self::P2PRecv),
            0x52 => Some(Self::P2PIsend),
            0x53 => Some(Self::P2PIrecv),
            0x54 => Some(Self::P2PWait),
            // Gradient Operations
            0x60 => Some(Self::BucketGradients),
            0x61 => Some(Self::GetGrad),
            0x62 => Some(Self::SetGrad),
            0x63 => Some(Self::ModuleBackward),
            0x64 => Some(Self::ZeroGrad),
            0x65 => Some(Self::ClipGradNorm),
            0x66 => Some(Self::JvpBegin),
            0x67 => Some(Self::JvpEnd),
            0x68 => Some(Self::GradCustom),
            0x69 => Some(Self::GradZeroTangent),
            0x6A => Some(Self::GradRecompute),
            // Actor/Mesh Operations
            0x70 => Some(Self::MeshSelect),
            0x71 => Some(Self::ActorNewId),
            0x72 => Some(Self::MeshCreate),
            0x73 => Some(Self::MeshShape),
            // RDMA Operations
            0x80 => Some(Self::RdmaCreateRef),
            0x81 => Some(Self::RdmaFetch),
            0x82 => Some(Self::RdmaWrite),
            0x83 => Some(Self::RdmaCheckValid),
            _ => None,
        }
    }

    /// Returns the byte value of this ML sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this ML sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Tokenizer Operations
            Self::TokenizerLoadBpe => "ML_TOKENIZER_LOAD_BPE",
            Self::TokenizerLoadPretrained => "ML_TOKENIZER_LOAD_PRETRAINED",
            Self::TokenizerEncode => "ML_TOKENIZER_ENCODE",
            Self::TokenizerDecode => "ML_TOKENIZER_DECODE",
            Self::TokenizerLoadSpm => "ML_TOKENIZER_LOAD_SPM",
            Self::TokenizerSpmEncode => "ML_TOKENIZER_SPM_ENCODE",
            Self::TokenizerSpmDecode => "ML_TOKENIZER_SPM_DECODE",
            // Sampling Operations
            Self::SampleTopP => "ML_SAMPLE_TOP_P",
            Self::SampleTemperature => "ML_SAMPLE_TEMPERATURE",
            Self::PagedAttention => "ML_PAGED_ATTENTION",
            Self::SampleTopK => "ML_SAMPLE_TOP_K",
            Self::SampleTopKTopP => "ML_SAMPLE_TOP_K_TOP_P",
            Self::RepetitionPenalty => "ML_REPETITION_PENALTY",
            // Inference Utility Operations
            Self::ParseToolCall => "ML_PARSE_TOOL_CALL",
            Self::FormatValue => "ML_FORMAT_VALUE",
            Self::QuantizedMatmul => "ML_QUANTIZED_MATMUL",
            Self::GenerateRequestId => "ML_GENERATE_REQUEST_ID",
            Self::JsonSchemaToJson => "ML_JSON_SCHEMA_TO_JSON",
            Self::FunctionSchemaToJson => "ML_FUNCTION_SCHEMA_TO_JSON",
            Self::ParseFunctionCalls => "ML_PARSE_FUNCTION_CALLS",
            Self::KvCacheOp => "ML_KV_CACHE_OP",
            Self::SpeculativeVerify => "ML_SPECULATIVE_VERIFY",
            // Distributed/Collective Operations
            Self::AllReduce => "ML_ALL_REDUCE",
            Self::AllGather => "ML_ALL_GATHER",
            Self::Broadcast => "ML_BROADCAST",
            Self::ReduceScatter => "ML_REDUCE_SCATTER",
            Self::Barrier => "ML_BARRIER",
            Self::PmapPsum => "ML_PMAP_PSUM",
            Self::PmapPmean => "ML_PMAP_PMEAN",
            Self::PmapPmax => "ML_PMAP_PMAX",
            Self::PmapAllGather => "ML_PMAP_ALL_GATHER",
            Self::VmapTransform => "ML_VMAP_TRANSFORM",
            Self::PmapTransform => "ML_PMAP_TRANSFORM",
            // Process Group Operations
            Self::DistWorldGroup => "ML_DIST_WORLD_GROUP",
            Self::DistNewGroup => "ML_DIST_NEW_GROUP",
            Self::DistGetRank => "ML_DIST_GET_RANK",
            Self::DistWorldSize => "ML_DIST_WORLD_SIZE",
            Self::DistLocalRank => "ML_DIST_LOCAL_RANK",
            // Point-to-Point Operations
            Self::P2PSend => "ML_P2P_SEND",
            Self::P2PRecv => "ML_P2P_RECV",
            Self::P2PIsend => "ML_P2P_ISEND",
            Self::P2PIrecv => "ML_P2P_IRECV",
            Self::P2PWait => "ML_P2P_WAIT",
            // Gradient Operations
            Self::BucketGradients => "ML_BUCKET_GRADIENTS",
            Self::GetGrad => "ML_GET_GRAD",
            Self::SetGrad => "ML_SET_GRAD",
            Self::ModuleBackward => "ML_MODULE_BACKWARD",
            Self::ZeroGrad => "ML_ZERO_GRAD",
            Self::ClipGradNorm => "ML_CLIP_GRAD_NORM",
            Self::JvpBegin => "ML_JVP_BEGIN",
            Self::JvpEnd => "ML_JVP_END",
            Self::GradCustom => "ML_GRAD_CUSTOM",
            Self::GradZeroTangent => "ML_GRAD_ZERO_TANGENT",
            Self::GradRecompute => "ML_GRAD_RECOMPUTE",
            // Actor/Mesh Operations
            Self::MeshSelect => "ML_MESH_SELECT",
            Self::ActorNewId => "ML_ACTOR_NEW_ID",
            Self::MeshCreate => "ML_MESH_CREATE",
            Self::MeshShape => "ML_MESH_SHAPE",
            // RDMA Operations
            Self::RdmaCreateRef => "ML_RDMA_CREATE_REF",
            Self::RdmaFetch => "ML_RDMA_FETCH",
            Self::RdmaWrite => "ML_RDMA_WRITE",
            Self::RdmaCheckValid => "ML_RDMA_CHECK_VALID",
        }
    }

    /// Returns the category of this ML sub-opcode.
    pub fn category(self) -> &'static str {
        match self as u8 {
            0x00..=0x0F => "Tokenizer Operations",
            0x10..=0x1F => "Sampling Operations",
            0x20..=0x2F => "Inference Utility Operations",
            0x30..=0x3F => "Distributed/Collective Operations",
            0x40..=0x4F => "Process Group Operations",
            0x50..=0x5F => "Point-to-Point Operations",
            0x60..=0x6F => "Gradient Operations",
            0x70..=0x7F => "Actor/Mesh Operations",
            0x80..=0x8F => "RDMA Operations",
            _ => "Unknown",
        }
    }

    /// Returns true if this is a collective operation.
    pub fn is_collective(self) -> bool {
        matches!(
            self,
            Self::AllReduce
                | Self::AllGather
                | Self::Broadcast
                | Self::ReduceScatter
                | Self::Barrier
                | Self::PmapPsum
                | Self::PmapPmean
                | Self::PmapPmax
                | Self::PmapAllGather
        )
    }

    /// Returns true if this is a point-to-point operation.
    pub fn is_p2p(self) -> bool {
        matches!(
            self,
            Self::P2PSend | Self::P2PRecv | Self::P2PIsend | Self::P2PIrecv | Self::P2PWait
        )
    }
}

// ============================================================================
// FFI Extended Sub-Opcodes
// ============================================================================

/// FFI extended sub-opcodes for use with `FfiExtended` (0xBC) prefix.
///
/// Provides comprehensive FFI operations for:
/// - Symbol resolution and caching
/// - C calling convention calls
/// - Platform-specific calling conventions (stdcall, sysv64)
/// - Variadic calls (printf-style)
/// - Value marshalling between Verum and C
/// - Error handling (errno)
///
/// # Encoding
///
/// ```text
/// [0xBC] [sub_opcode:u8] [operands...]
/// ```
///
/// # Example
///
/// ```text
/// // Call getpid() from libc
/// FfiExtended CallFfiC symbol_idx:u32, arg_count:0, ret_reg:r0
///
/// // Call printf with variadic args
/// FfiExtended CallFfiVariadic symbol_idx:u32, arg_count:2, ret_reg:r0
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum FfiSubOpcode {
    // ========================================================================
    // Symbol Resolution (0x00-0x0F)
    // ========================================================================
    /// Resolve FFI symbol and cache address.
    ///
    /// Format: `dst:reg, symbol_idx:u32`
    /// Returns: Pointer to resolved symbol (cached for subsequent calls)
    LoadSymbol = 0x00,

    /// Get library handle.
    ///
    /// Format: `dst:reg, library_idx:u16`
    /// Returns: Library handle or null if not loaded
    GetLibrary = 0x01,

    /// Check if symbol is resolved.
    ///
    /// Format: `dst:reg, symbol_idx:u32`
    /// Returns: true if symbol is resolved, false otherwise
    IsSymbolResolved = 0x02,

    // ========================================================================
    // C Calling Convention (0x10-0x1F)
    // ========================================================================
    /// Call with C calling convention.
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// The C calling convention is the default on most Unix systems.
    CallFfiC = 0x10,

    /// Call with stdcall convention (Windows).
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// Callee cleans up the stack.
    CallFfiStdcall = 0x11,

    /// Call with System V AMD64 ABI.
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// First 6 args in registers, rest on stack.
    CallFfiSysV64 = 0x12,

    /// Call with fastcall convention (Windows).
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// First 2 args in registers (ECX, EDX).
    CallFfiFastcall = 0x13,

    /// Call with variadic convention (printf-style).
    ///
    /// Format: `symbol_idx:u32, fixed_count:u8, variadic_count:u8, ret_reg:reg, [arg_regs...]`
    /// First fixed_count args are typed, remaining are variadic.
    CallFfiVariadic = 0x14,

    /// Indirect call through function pointer.
    ///
    /// Format: `ptr_reg:reg, signature_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// Calls the function at the address in ptr_reg.
    CallFfiIndirect = 0x15,

    /// Call with ARM64 AAPCS calling convention.
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// ARM64 Procedure Call Standard - first 8 args in X0-X7/V0-V7 registers.
    CallFfiAarch64 = 0x16,

    /// Call with Windows ARM64 calling convention.
    ///
    /// Format: `symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...]`
    /// Windows ARM64 follows a variant of AAPCS with some differences.
    CallFfiWin64Arm64 = 0x17,

    // ========================================================================
    // Marshalling (0x20-0x2F)
    // ========================================================================
    /// Marshal Verum value to C representation.
    ///
    /// Format: `dst:reg, src:reg, c_type:u8`
    /// c_type: 0=i8, 1=i16, 2=i32, 3=i64, 4=u8, 5=u16, 6=u32, 7=u64,
    ///         8=f32, 9=f64, 10=ptr, 11=bool, 12=void
    MarshalToC = 0x20,

    /// Marshal C value to Verum representation.
    ///
    /// Format: `dst:reg, src:reg, c_type:u8`
    /// Converts C representation back to Verum value.
    MarshalFromC = 0x21,

    /// Marshal string to C (null-terminated).
    ///
    /// Format: `dst:reg, src:reg`
    /// Returns pointer to null-terminated UTF-8 string (caller must free).
    StringToC = 0x22,

    /// Marshal C string to Verum Text.
    ///
    /// Format: `dst:reg, src:reg`
    /// Copies null-terminated C string into Verum Text.
    StringFromC = 0x23,

    /// Marshal array to C pointer.
    ///
    /// Format: `ptr:reg, len:reg, src:reg`
    /// Returns pointer to array data and length.
    ArrayToC = 0x24,

    /// Marshal C array to Verum List.
    ///
    /// Format: `dst:reg, ptr:reg, len:reg, elem_type:u8`
    /// Copies C array into Verum List.
    ArrayFromC = 0x25,

    /// Marshal struct to C layout.
    ///
    /// Format: `dst:reg, src:reg, layout_idx:u32`
    /// Converts Verum record to C struct layout.
    StructToC = 0x26,

    /// Marshal C struct to Verum record.
    ///
    /// Format: `dst:reg, src:reg, layout_idx:u32`
    /// Converts C struct to Verum record.
    StructFromC = 0x27,

    // ========================================================================
    // Error Handling (0x30-0x3F)
    // ========================================================================
    /// Get errno value.
    ///
    /// Format: `dst:reg`
    /// Returns current errno value.
    GetErrno = 0x30,

    /// Set errno value.
    ///
    /// Format: `value:reg`
    /// Sets errno to the specified value.
    SetErrno = 0x31,

    /// Clear errno.
    ///
    /// Format: (no operands)
    /// Sets errno to 0.
    ClearErrno = 0x32,

    /// Get last Win32 error (Windows).
    ///
    /// Format: `dst:reg`
    /// Returns GetLastError() value.
    GetLastError = 0x33,

    // ========================================================================
    // Memory Operations (0x40-0x4F)
    // ========================================================================
    /// Allocate C memory (malloc).
    ///
    /// Format: `dst:reg, size:reg`
    /// Returns pointer to allocated memory.
    CAlloc = 0x40,

    /// Free C memory (free).
    ///
    /// Format: `ptr:reg`
    /// Frees memory allocated by CAlloc.
    CFree = 0x41,

    /// Reallocate C memory (realloc).
    ///
    /// Format: `dst:reg, ptr:reg, size:reg`
    /// Returns pointer to reallocated memory.
    CRealloc = 0x42,

    /// Copy C memory (memcpy).
    ///
    /// Format: `dst:reg, src:reg, size:reg`
    CMemcpy = 0x43,

    /// Set C memory (memset).
    ///
    /// Format: `dst:reg, value:reg, size:reg`
    CMemset = 0x44,

    /// Move C memory (memmove) - handles overlapping regions.
    ///
    /// Format: `dst:reg, src:reg, size:reg`
    CMemmove = 0x45,

    /// Compare C memory (memcmp).
    ///
    /// Format: `dst:reg, ptr1:reg, ptr2:reg, size:reg`
    /// Returns: negative if ptr1 < ptr2, 0 if equal, positive if ptr1 > ptr2
    CMemcmp = 0x46,

    /// Generate cryptographically secure random u64.
    ///
    /// Format: `dst:reg`
    /// Uses platform-specific secure random:
    /// - macOS: getentropy()
    /// - Linux: getrandom syscall
    /// - Windows: BCryptGenRandom
    RandomU64 = 0x47,

    /// Generate random float in [0, 1).
    ///
    /// Format: `dst:reg`
    /// Uses RandomU64 internally with IEEE 754 conversion.
    RandomFloat = 0x48,

    /// Allocate a byte array (contiguous bytes, not Values).
    ///
    /// Format: `dst:reg, size:reg, init:reg`
    /// Allocates `size` bytes of contiguous memory with TypeId::U8.
    /// Each byte is initialized to `init` value.
    ///
    /// This is used for `let buf: [Byte; N] = uninit();` or `[Byte; N] = zeroed();`
    /// where we need true byte arrays, not Value arrays.
    NewByteArray = 0x49,

    /// Get element address in a byte array.
    ///
    /// Format: `dst:reg, arr:reg, idx:reg`
    /// Computes the memory address of the element at index `idx` in byte array `arr`.
    /// Returns: `dst = arr_ptr + OBJECT_HEADER_SIZE + idx`
    ///
    /// This is used for `&mut buf[idx] as *mut Byte` to get the actual memory address
    /// of a byte array element, rather than fetching its value (which GetE does).
    ByteArrayElementAddr = 0x4A,

    /// Load a byte from a byte array.
    ///
    /// Format: `dst:reg, arr:reg, idx:reg`
    /// Loads the byte at index `idx` from byte array `arr` into `dst`.
    ///
    /// This provides efficient single-byte access to byte arrays without
    /// computing element addresses.
    ByteArrayLoad = 0x4B,

    /// Store a byte to a byte array.
    ///
    /// Format: `arr:reg, idx:reg, val:reg`
    /// Stores the low 8 bits of `val` to byte array `arr` at index `idx`.
    ///
    /// This provides efficient single-byte writes to byte arrays without
    /// computing element addresses.
    ByteArrayStore = 0x4C,

    /// Get element address for typed array (with element size).
    ///
    /// Format: `dst:reg, arr:reg, idx:reg, elem_size:u8`
    /// Returns: Pointer to arr[idx] = base_addr + (idx * elem_size)
    ///
    /// This is a generalization of ByteArrayElementAddr for arrays with
    /// elements larger than 1 byte (e.g., [UInt64; N] where elem_size=8).
    TypedArrayElementAddr = 0x4D,

    /// Create new typed array with element size.
    ///
    /// Format: `dst:reg, count:reg, elem_size:u8, init:reg`
    /// Allocates: count * elem_size bytes of memory
    /// Initializes: All elements to init value (cast to element type)
    NewTypedArray = 0x4E,

    /// Get raw address of a struct field.
    ///
    /// Format: `dst:reg, obj:reg, field_offset:u16`
    /// Returns: `obj_heap_ptr + OBJECT_HEADER_SIZE + field_offset`
    ///   (or, for register-encoded scalars stored in a Value-typed
    ///    field, the address of the Value's u64 storage so atomic
    ///    sub-byte reads of the inline-int payload land correctly
    ///    on little-endian targets)
    ///
    /// Generalises ByteArrayElementAddr / TypedArrayElementAddr to
    /// the struct-field surface so `&self.field as *const T` lowers
    /// to a real heap address — required by every atomic stdlib op
    /// (AtomicU8 / AtomicU16 / AtomicU32) that takes
    /// `&self.value as *const Byte` and feeds it to the typed
    /// `atomic_load_*` / `atomic_cas_*` intrinsics.  Without this,
    /// `&self.value` produces a register-encoded CBGR ref whose
    /// bit-pattern is meaningless when cast to a raw pointer (the
    /// historical bug — every Tier-0 atomic was a silent no-op).
    ///
    /// # Safety
    /// Caller must ensure obj is a valid heap-allocated object and
    /// field_offset stays within the object's data section.
    StructFieldAddr = 0x4F,

    // ========================================================================
    // Callback Support (0x50-0x5F)
    // ========================================================================
    /// Create callback trampoline.
    ///
    /// Format: `dst:reg, fn_id:u32, signature_idx:u32`
    /// Creates a C-callable function pointer that invokes Verum function.
    CreateCallback = 0x50,

    /// Free callback trampoline.
    ///
    /// Format: `trampoline:reg`
    /// Frees resources associated with callback.
    FreeCallback = 0x51,

    // ========================================================================
    // Raw Pointer Operations (0x60-0x6F)
    // ========================================================================
    // These operations bypass CBGR validation for FFI-returned raw pointers.
    // They provide direct memory access similar to C pointers.
    //
    // SAFETY: These operations are inherently unsafe. The caller must ensure:
    // - The pointer is valid and points to accessible memory
    // - The memory is properly aligned for the target type
    // - The memory is not freed or moved during the operation
    // - No data races occur when using mutable operations
    //
    // These are semantically equivalent to Rust's `*const T` and `*mut T`.
    // ========================================================================

    /// Read value through raw pointer (no CBGR validation).
    ///
    /// Format: `dst:reg, ptr:reg, size:u8`
    /// size: 1=i8, 2=i16, 4=i32, 8=i64 (interpreted as signed)
    ///
    /// Reads a primitive value from the memory address in `ptr`.
    /// This bypasses CBGR validation and directly accesses memory.
    ///
    /// # Safety
    /// The pointer must be valid and properly aligned.
    DerefRaw = 0x60,

    /// Write value through raw pointer (no CBGR validation).
    ///
    /// Format: `ptr:reg, value:reg, size:u8`
    /// size: 1=i8, 2=i16, 4=i32, 8=i64
    ///
    /// Writes a primitive value to the memory address in `ptr`.
    /// This bypasses CBGR validation and directly accesses memory.
    ///
    /// # Safety
    /// The pointer must be valid and properly aligned.
    /// The memory must be writable.
    DerefMutRaw = 0x61,

    /// Read pointer through raw pointer (for pointer-to-pointer).
    ///
    /// Format: `dst:reg, ptr:reg`
    ///
    /// Reads a pointer value from the memory address in `ptr`.
    DerefRawPtr = 0x62,

    /// Pointer arithmetic: add offset.
    ///
    /// Format: `dst:reg, ptr:reg, offset:reg`
    ///
    /// Computes `ptr + offset` for raw pointer arithmetic.
    /// The offset is in bytes.
    PtrAdd = 0x63,

    /// Pointer arithmetic: subtract offset.
    ///
    /// Format: `dst:reg, ptr:reg, offset:reg`
    ///
    /// Computes `ptr - offset` for raw pointer arithmetic.
    PtrSub = 0x64,

    /// Pointer difference.
    ///
    /// Format: `dst:reg, ptr1:reg, ptr2:reg`
    ///
    /// Computes `ptr1 - ptr2` (difference in bytes).
    PtrDiff = 0x65,

    /// Check if pointer is null.
    ///
    /// Format: `dst:reg, ptr:reg`
    ///
    /// Sets `dst` to true if `ptr` is null, false otherwise.
    PtrIsNull = 0x66,

    // ========================================================================
    // Time Operations (0x70-0x7F)
    // ========================================================================
    /// Get monotonic time in nanoseconds.
    ///
    /// Format: `dst:reg`
    /// Returns: Current monotonic clock time in nanoseconds (i64).
    /// Uses platform-specific monotonic clock (CLOCK_MONOTONIC on macOS/Linux).
    TimeMonotonicNanos = 0x70,

    /// Get realtime (wall clock) in nanoseconds since Unix epoch.
    ///
    /// Format: `dst:reg`
    /// Returns: Nanoseconds since 1970-01-01T00:00:00Z (i64).
    TimeRealtimeNanos = 0x71,

    /// Get raw monotonic time in nanoseconds (not NTP-adjusted).
    ///
    /// Format: `dst:reg`
    /// Returns: Raw monotonic clock time in nanoseconds (i64).
    /// Uses CLOCK_MONOTONIC_RAW on macOS/Linux.
    TimeMonotonicRawNanos = 0x72,

    /// Sleep for specified nanoseconds.
    ///
    /// Format: `nanos:reg`
    /// Sleeps the current thread for the specified duration.
    TimeSleepNanos = 0x73,

    /// Get thread CPU time in nanoseconds.
    ///
    /// Format: `dst:reg`
    /// Returns: Thread CPU time in nanoseconds (i64).
    /// Uses CLOCK_THREAD_CPUTIME_ID.
    TimeThreadCpuNanos = 0x74,

    /// Get process CPU time in nanoseconds.
    ///
    /// Format: `dst:reg`
    /// Returns: Process CPU time in nanoseconds (i64).
    /// Uses CLOCK_PROCESS_CPUTIME_ID.
    TimeProcessCpuNanos = 0x75,

    // ========================================================================
    // System Call Operations (0x80-0x8F)
    // ========================================================================
    /// Get process ID.
    ///
    /// Format: `dst:reg`
    /// Returns: Process ID as i64.
    SysGetpid = 0x80,

    /// Get thread ID.
    ///
    /// Format: `dst:reg`
    /// Returns: Thread ID as u64.
    SysGettid = 0x81,

    /// Memory map (mmap).
    ///
    /// Format: `dst:reg, addr:reg, len:reg, prot:reg, flags:reg, fd:reg, offset:reg`
    /// Returns: Result variant (Ok=pointer, Err=OSError).
    SysMmap = 0x82,

    /// Memory unmap (munmap).
    ///
    /// Format: `dst:reg, addr:reg, len:reg`
    /// Returns: Result variant (Ok=unit, Err=OSError).
    SysMunmap = 0x83,

    /// Memory advise (madvise).
    ///
    /// Format: `dst:reg, addr:reg, len:reg, advice:reg`
    /// Returns: Result variant (Ok=unit, Err=OSError).
    SysMadvise = 0x84,

    /// Get entropy (getentropy).
    ///
    /// Format: `dst:reg, buf:reg, len:reg`
    /// Returns: Result variant (Ok=unit, Err=OSError).
    SysGetentropy = 0x85,

    // =========================================================================
    // Mach Kernel Operations (macOS)
    // =========================================================================

    /// Mach vm_allocate (safe wrapper).
    ///
    /// Format: `dst:reg, size:reg, anywhere:reg`
    /// Returns: Result variant (Ok=VmAddress as Int, Err=KernReturn as Int).
    MachVmAllocate = 0x90,

    /// Mach vm_deallocate (safe wrapper).
    ///
    /// Format: `dst:reg, addr:reg, size:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachVmDeallocate = 0x91,

    /// Mach vm_protect (safe wrapper).
    ///
    /// Format: `dst:reg, addr:reg, size:reg, prot:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachVmProtect = 0x92,

    /// Mach semaphore_create (safe wrapper).
    ///
    /// Format: `dst:reg, initial_value:reg`
    /// Returns: Result variant (Ok=SemaphoreT as Int, Err=KernReturn as Int).
    MachSemCreate = 0x93,

    /// Mach semaphore_destroy (safe wrapper).
    ///
    /// Format: `dst:reg, sem:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachSemDestroy = 0x94,

    /// Mach semaphore_signal (safe wrapper).
    ///
    /// Format: `dst:reg, sem:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachSemSignal = 0x95,

    /// Mach semaphore_wait (safe wrapper).
    ///
    /// Format: `dst:reg, sem:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachSemWait = 0x96,

    /// Mach mach_error_string.
    ///
    /// Format: `dst:reg, kern_return:reg`
    /// Returns: Text (string).
    MachErrorString = 0x97,

    /// Mach mach_wait_until (sleep until deadline).
    ///
    /// Format: `dst:reg, deadline:reg`
    /// Returns: Result variant (Ok=unit, Err=KernReturn as Int).
    MachSleepUntil = 0x98,

    // ========================================================================
    // CBGR Memory Operations (0xA0-0xAF) — tracked allocation/deallocation
    // with generation-and-epoch metadata. Distinct from the raw C allocator
    // at 0x40-0x42: these return a Result tuple `(ptr, generation, epoch)`
    // and register the allocation in the CBGR validation table.
    // ========================================================================
    /// Allocate memory with CBGR tracking.
    ///
    /// Format: `dst:reg, size:reg, align:reg`
    /// Returns: Result tuple `(ptr, generation, epoch)` or AllocError.
    CbgrAlloc = 0xA0,

    /// Allocate zeroed memory with CBGR tracking.
    ///
    /// Format: `dst:reg, size:reg, align:reg`
    /// Returns: Result tuple `(ptr, generation, epoch)` or AllocError.
    CbgrAllocZeroed = 0xA1,

    /// Deallocate memory previously allocated via `CbgrAlloc`.
    ///
    /// Format: `dst:reg, ptr:reg, size:reg, align:reg`
    /// Returns: Result unit or AllocError.
    CbgrDealloc = 0xA2,
}

impl FfiSubOpcode {
    /// Creates an FFI sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Symbol Resolution
            0x00 => Some(Self::LoadSymbol),
            0x01 => Some(Self::GetLibrary),
            0x02 => Some(Self::IsSymbolResolved),
            // C Calling Convention
            0x10 => Some(Self::CallFfiC),
            0x11 => Some(Self::CallFfiStdcall),
            0x12 => Some(Self::CallFfiSysV64),
            0x13 => Some(Self::CallFfiFastcall),
            0x14 => Some(Self::CallFfiVariadic),
            0x15 => Some(Self::CallFfiIndirect),
            0x16 => Some(Self::CallFfiAarch64),
            0x17 => Some(Self::CallFfiWin64Arm64),
            // Marshalling
            0x20 => Some(Self::MarshalToC),
            0x21 => Some(Self::MarshalFromC),
            0x22 => Some(Self::StringToC),
            0x23 => Some(Self::StringFromC),
            0x24 => Some(Self::ArrayToC),
            0x25 => Some(Self::ArrayFromC),
            0x26 => Some(Self::StructToC),
            0x27 => Some(Self::StructFromC),
            // Error Handling
            0x30 => Some(Self::GetErrno),
            0x31 => Some(Self::SetErrno),
            0x32 => Some(Self::ClearErrno),
            0x33 => Some(Self::GetLastError),
            // Memory Operations
            0x40 => Some(Self::CAlloc),
            0x41 => Some(Self::CFree),
            0x42 => Some(Self::CRealloc),
            0x43 => Some(Self::CMemcpy),
            0x44 => Some(Self::CMemset),
            0x45 => Some(Self::CMemmove),
            0x46 => Some(Self::CMemcmp),
            0x47 => Some(Self::RandomU64),
            0x48 => Some(Self::RandomFloat),
            0x49 => Some(Self::NewByteArray),
            0x4A => Some(Self::ByteArrayElementAddr),
            0x4B => Some(Self::ByteArrayLoad),
            0x4C => Some(Self::ByteArrayStore),
            0x4D => Some(Self::TypedArrayElementAddr),
            0x4E => Some(Self::NewTypedArray),
            0x4F => Some(Self::StructFieldAddr),
            // Callback Support
            0x50 => Some(Self::CreateCallback),
            0x51 => Some(Self::FreeCallback),
            // Raw Pointer Operations
            0x60 => Some(Self::DerefRaw),
            0x61 => Some(Self::DerefMutRaw),
            0x62 => Some(Self::DerefRawPtr),
            0x63 => Some(Self::PtrAdd),
            0x64 => Some(Self::PtrSub),
            0x65 => Some(Self::PtrDiff),
            0x66 => Some(Self::PtrIsNull),
            // Time Operations
            0x70 => Some(Self::TimeMonotonicNanos),
            0x71 => Some(Self::TimeRealtimeNanos),
            0x72 => Some(Self::TimeMonotonicRawNanos),
            0x73 => Some(Self::TimeSleepNanos),
            0x74 => Some(Self::TimeThreadCpuNanos),
            0x75 => Some(Self::TimeProcessCpuNanos),
            // System Call Operations
            0x80 => Some(Self::SysGetpid),
            0x81 => Some(Self::SysGettid),
            0x82 => Some(Self::SysMmap),
            0x83 => Some(Self::SysMunmap),
            0x84 => Some(Self::SysMadvise),
            0x85 => Some(Self::SysGetentropy),
            // Mach Kernel Operations
            0x90 => Some(Self::MachVmAllocate),
            0x91 => Some(Self::MachVmDeallocate),
            0x92 => Some(Self::MachVmProtect),
            0x93 => Some(Self::MachSemCreate),
            0x94 => Some(Self::MachSemDestroy),
            0x95 => Some(Self::MachSemSignal),
            0x96 => Some(Self::MachSemWait),
            0x97 => Some(Self::MachErrorString),
            0x98 => Some(Self::MachSleepUntil),
            // CBGR Memory Operations
            0xA0 => Some(Self::CbgrAlloc),
            0xA1 => Some(Self::CbgrAllocZeroed),
            0xA2 => Some(Self::CbgrDealloc),
            _ => None,
        }
    }

    /// Returns the byte value of this FFI sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this FFI sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::LoadSymbol => "FFI_LOAD_SYMBOL",
            Self::GetLibrary => "FFI_GET_LIBRARY",
            Self::IsSymbolResolved => "FFI_IS_RESOLVED",
            Self::CallFfiC => "FFI_CALL_C",
            Self::CallFfiStdcall => "FFI_CALL_STDCALL",
            Self::CallFfiSysV64 => "FFI_CALL_SYSV64",
            Self::CallFfiFastcall => "FFI_CALL_FASTCALL",
            Self::CallFfiVariadic => "FFI_CALL_VARIADIC",
            Self::CallFfiIndirect => "FFI_CALL_INDIRECT",
            Self::CallFfiAarch64 => "FFI_CALL_AARCH64",
            Self::CallFfiWin64Arm64 => "FFI_CALL_WIN64_ARM64",
            Self::MarshalToC => "FFI_MARSHAL_TO_C",
            Self::MarshalFromC => "FFI_MARSHAL_FROM_C",
            Self::StringToC => "FFI_STRING_TO_C",
            Self::StringFromC => "FFI_STRING_FROM_C",
            Self::ArrayToC => "FFI_ARRAY_TO_C",
            Self::ArrayFromC => "FFI_ARRAY_FROM_C",
            Self::StructToC => "FFI_STRUCT_TO_C",
            Self::StructFromC => "FFI_STRUCT_FROM_C",
            Self::GetErrno => "FFI_GET_ERRNO",
            Self::SetErrno => "FFI_SET_ERRNO",
            Self::ClearErrno => "FFI_CLEAR_ERRNO",
            Self::GetLastError => "FFI_GET_LAST_ERROR",
            Self::CAlloc => "FFI_C_ALLOC",
            Self::CFree => "FFI_C_FREE",
            Self::CRealloc => "FFI_C_REALLOC",
            Self::CMemcpy => "FFI_C_MEMCPY",
            Self::CMemset => "FFI_C_MEMSET",
            Self::CMemmove => "FFI_C_MEMMOVE",
            Self::CMemcmp => "FFI_C_MEMCMP",
            Self::RandomU64 => "FFI_RANDOM_U64",
            Self::RandomFloat => "FFI_RANDOM_FLOAT",
            Self::NewByteArray => "FFI_NEW_BYTE_ARRAY",
            Self::ByteArrayElementAddr => "FFI_BYTE_ARRAY_ELEM_ADDR",
            Self::ByteArrayLoad => "FFI_BYTE_ARRAY_LOAD",
            Self::ByteArrayStore => "FFI_BYTE_ARRAY_STORE",
            Self::TypedArrayElementAddr => "FFI_TYPED_ARRAY_ELEM_ADDR",
            Self::NewTypedArray => "FFI_NEW_TYPED_ARRAY",
            Self::StructFieldAddr => "FFI_STRUCT_FIELD_ADDR",
            Self::CreateCallback => "FFI_CREATE_CALLBACK",
            Self::FreeCallback => "FFI_FREE_CALLBACK",
            Self::DerefRaw => "FFI_DEREF_RAW",
            Self::DerefMutRaw => "FFI_DEREF_MUT_RAW",
            Self::DerefRawPtr => "FFI_DEREF_RAW_PTR",
            Self::PtrAdd => "FFI_PTR_ADD",
            Self::PtrSub => "FFI_PTR_SUB",
            Self::PtrDiff => "FFI_PTR_DIFF",
            Self::PtrIsNull => "FFI_PTR_IS_NULL",
            Self::TimeMonotonicNanos => "TIME_MONOTONIC_NANOS",
            Self::TimeRealtimeNanos => "TIME_REALTIME_NANOS",
            Self::TimeMonotonicRawNanos => "TIME_MONOTONIC_RAW_NANOS",
            Self::TimeSleepNanos => "TIME_SLEEP_NANOS",
            Self::TimeThreadCpuNanos => "TIME_THREAD_CPU_NANOS",
            Self::TimeProcessCpuNanos => "TIME_PROCESS_CPU_NANOS",
            Self::SysGetpid => "SYS_GETPID",
            Self::SysGettid => "SYS_GETTID",
            Self::SysMmap => "SYS_MMAP",
            Self::SysMunmap => "SYS_MUNMAP",
            Self::SysMadvise => "SYS_MADVISE",
            Self::SysGetentropy => "SYS_GETENTROPY",
            Self::MachVmAllocate => "MACH_VM_ALLOCATE",
            Self::MachVmDeallocate => "MACH_VM_DEALLOCATE",
            Self::MachVmProtect => "MACH_VM_PROTECT",
            Self::MachSemCreate => "MACH_SEM_CREATE",
            Self::MachSemDestroy => "MACH_SEM_DESTROY",
            Self::MachSemSignal => "MACH_SEM_SIGNAL",
            Self::MachSemWait => "MACH_SEM_WAIT",
            Self::MachErrorString => "MACH_ERROR_STRING",
            Self::MachSleepUntil => "MACH_SLEEP_UNTIL",
            Self::CbgrAlloc => "CBGR_ALLOC",
            Self::CbgrAllocZeroed => "CBGR_ALLOC_ZEROED",
            Self::CbgrDealloc => "CBGR_DEALLOC",
        }
    }

    /// Returns the category name for this sub-opcode range.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "Symbol Resolution",
            0x10..=0x1F => "Calling Convention",
            0x20..=0x2F => "Marshalling",
            0x30..=0x3F => "Error Handling",
            0x40..=0x4F => "Memory Operations",
            0x50..=0x5F => "Callback Support",
            0x60..=0x6F => "Raw Pointer Operations",
            0x70..=0x7F => "Time Operations",
            0x80..=0x8F => "System Call Operations",
            0x90..=0x9F => "Mach Kernel Operations",
            _ => "Unknown",
        }
    }

    /// Returns true if this operation performs a call.
    pub fn is_call(self) -> bool {
        matches!(
            self,
            Self::CallFfiC
                | Self::CallFfiStdcall
                | Self::CallFfiSysV64
                | Self::CallFfiFastcall
                | Self::CallFfiVariadic
                | Self::CallFfiIndirect
        )
    }

    /// Returns true if this operation marshals data.
    pub fn is_marshal(self) -> bool {
        matches!(
            self,
            Self::MarshalToC
                | Self::MarshalFromC
                | Self::StringToC
                | Self::StringFromC
                | Self::ArrayToC
                | Self::ArrayFromC
                | Self::StructToC
                | Self::StructFromC
        )
    }

    /// Returns true if this operation allocates memory.
    pub fn allocates(self) -> bool {
        matches!(self, Self::CAlloc | Self::CRealloc | Self::CreateCallback)
    }

    /// Returns true if this operation frees memory.
    pub fn deallocates(self) -> bool {
        matches!(self, Self::CFree | Self::FreeCallback)
    }
}

// ============================================================================
// Arithmetic Extended Sub-Opcodes
// ============================================================================

/// Arithmetic extended sub-opcodes for use with `ArithExtended` (0xBD) prefix.
///
/// This provides extended arithmetic operations that were previously misplaced
/// in FfiSubOpcode space. Moving them to dedicated ArithSubOpcode space provides:
/// - Clean semantic separation (arithmetic vs FFI)
/// - Optimized dispatch path for arithmetic operations
/// - Room for future expansion (saturating, SIMD, etc.)
///
/// # Sub-opcode Ranges
///
/// - 0x00-0x0F: Checked arithmetic (returns Maybe<T>)
/// - 0x10-0x1F: Overflowing arithmetic (returns (result, overflow_flag))
/// - 0x20-0x2F: Polymorphic arithmetic (type-dispatched)
/// - 0x30-0x3F: Reserved for saturating arithmetic
/// - 0x40-0x4F: Reserved for wrapping arithmetic
///
/// # Encoding
///
/// ```text
/// [0xBD] [sub_opcode:u8] [operands...]
/// ```
///
/// # Example
///
/// ```text
/// // Checked addition: dst = Some(a + b) or None if overflow
/// ArithExtended CheckedAddI dst:r0, a:r1, b:r2
///
/// // Overflowing multiplication: (result, did_overflow) = a * b
/// ArithExtended OverflowingMulI dst:r3, a:r4, b:r5
///
/// // Polymorphic addition (dispatches based on operand type)
/// ArithExtended PolyAdd dst:r6, a:r7, b:r8
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ArithSubOpcode {
    // ========================================================================
    // Checked Arithmetic (0x00-0x0F) - Returns Maybe<T>
    // ========================================================================
    /// Checked integer addition returning Maybe<Int>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no overflow, `None` if overflow.
    /// Uses Rust's `checked_add` internally for correct overflow detection.
    CheckedAddI = 0x00,

    /// Checked integer subtraction returning Maybe<Int>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no overflow, `None` if overflow.
    CheckedSubI = 0x01,

    /// Checked integer multiplication returning Maybe<Int>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no overflow, `None` if overflow.
    CheckedMulI = 0x02,

    /// Checked integer division returning Maybe<Int>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if divisor != 0 and no overflow, `None` otherwise.
    /// Handles both division by zero and MIN / -1 overflow.
    CheckedDivI = 0x03,

    /// Checked unsigned integer addition returning Maybe<UInt64>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no overflow, `None` if overflow.
    /// Uses Rust's u64::checked_add internally for correct unsigned overflow detection.
    CheckedAddU = 0x04,

    /// Checked unsigned integer subtraction returning Maybe<UInt64>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no underflow, `None` if underflow.
    CheckedSubU = 0x05,

    /// Checked unsigned integer multiplication returning Maybe<UInt64>.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns `Some(result)` if no overflow, `None` if overflow.
    CheckedMulU = 0x06,

    /// Checked signed integer negation returning Maybe<T>.
    ///
    /// Format: `dst:reg, src:reg, width:u8, signed:u8`
    ///
    /// Returns `Some(-src)` for every value EXCEPT signed `T::MIN`,
    /// for which the mathematical result `|T::MIN|` is unrepresentable
    /// in two's complement. `None` is returned for the unrepresentable
    /// case.  The closes the gap where `core/intrinsics/arithmetic.vr`
    /// declared `checked_neg<T>` but the compiler had no lowering for
    /// it (calls would panic at codegen).
    CheckedNeg = 0x07,

    /// Checked signed integer absolute value returning Maybe<T>.
    ///
    /// Format: `dst:reg, src:reg, width:u8, signed:u8`
    ///
    /// Returns `Some(|src|)` for every value EXCEPT signed `T::MIN`
    /// (same overflow as `CheckedNeg`).  Symmetric with `CheckedNeg`;
    /// previously absent from both the .vr surface and the compiler
    /// — closes the API completeness gap.
    CheckedAbs = 0x08,

    // ========================================================================
    // Overflowing Arithmetic (0x10-0x1F) - Returns (result, overflow_flag)
    // ========================================================================
    /// Overflowing integer addition returning (result, overflowed).
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns a tuple (wrapped_result: Int, did_overflow: Bool).
    /// Always produces a result using wrapping semantics.
    OverflowingAddI = 0x10,

    /// Overflowing integer subtraction returning (result, overflowed).
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns a tuple (wrapped_result: Int, did_overflow: Bool).
    OverflowingSubI = 0x11,

    /// Overflowing integer multiplication returning (result, overflowed).
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Returns a tuple (wrapped_result: Int, did_overflow: Bool).
    OverflowingMulI = 0x12,

    // ========================================================================
    // Polymorphic Arithmetic (0x20-0x2F) - Type-dispatched
    // ========================================================================
    // High-performance type-dispatched arithmetic for generic intrinsics.
    // These check the first operand's type at runtime and dispatch to the
    // appropriate integer or float operation. Overhead: ~1-2 cycles for type check.

    /// Polymorphic addition - dispatches to AddI or AddF based on operand type.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    PolyAdd = 0x20,

    /// Polymorphic subtraction - dispatches to SubI or SubF based on operand type.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    PolySub = 0x21,

    /// Polymorphic multiplication - dispatches to MulI or MulF based on operand type.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    PolyMul = 0x22,

    /// Polymorphic division - dispatches to DivI or DivF based on operand type.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    PolyDiv = 0x23,

    /// Polymorphic negation - dispatches to NegI or NegF based on operand type.
    ///
    /// Format: `dst:reg, src:reg`
    PolyNeg = 0x24,

    /// Polymorphic remainder - dispatches to ModI or ModF based on operand type.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    PolyRem = 0x25,

    /// Polymorphic absolute value - works for all signed numeric types.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Returns |src|. For integers, uses wrapping_abs to handle MIN value.
    PolyAbs = 0x26,

    /// Polymorphic signum - returns -1, 0, or 1 based on sign.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Returns -1 if src < 0, 0 if src == 0, 1 if src > 0.
    PolySignum = 0x27,

    /// Polymorphic minimum - returns the smaller of two values.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Works for all Ord types (integers and floats).
    PolyMin = 0x28,

    /// Polymorphic maximum - returns the larger of two values.
    ///
    /// Format: `dst:reg, a:reg, b:reg`
    ///
    /// Works for all Ord types (integers and floats).
    PolyMax = 0x29,

    /// Polymorphic clamp - clamps value to a range [min, max].
    ///
    /// Format: `dst:reg, val:reg, min:reg, max:reg`
    ///
    /// Returns min if val < min, max if val > max, otherwise val.
    PolyClamp = 0x2A,

    // ========================================================================
    // Saturating Arithmetic (0x30-0x3F) - Clamps to type bounds
    // ========================================================================
    // Operations that saturate at MIN/MAX instead of wrapping on overflow.
    // Format includes bit-width: `dst:reg, a:reg, b:reg, width:u8`
    // Width values: 8, 16, 32, 64 (128 for Int128/UInt128)

    /// Saturating addition - clamps to MAX on overflow, MIN on underflow.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8, signed:u8`
    SaturatingAdd = 0x30,

    /// Saturating subtraction - clamps to MIN on underflow, MAX on overflow.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8, signed:u8`
    SaturatingSub = 0x31,

    /// Saturating signed negation - clamps `T::MIN` to `T::MAX`.
    ///
    /// Format: `dst:reg, src:reg, width:u8, signed:u8`
    ///
    /// Mathematically `-T::MIN = T::MAX + 1` is unrepresentable in
    /// two's complement; rather than wrapping (WrappingNeg) or
    /// returning Maybe<T> (CheckedNeg), this saturates to `T::MAX`
    /// — the safe-but-lossy choice for code that prefers a
    /// definite value over an Option.
    SaturatingNeg = 0x33,

    /// Saturating signed absolute value - clamps `T::MIN` to `T::MAX`.
    ///
    /// Format: `dst:reg, src:reg, width:u8, signed:u8`
    ///
    /// `|T::MIN|` overflows for the same reason as `-T::MIN`;
    /// saturates to `T::MAX` instead of wrapping or panicking.
    SaturatingAbs = 0x34,

    /// Saturating multiplication - clamps to MAX/MIN on overflow.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8, signed:u8`
    SaturatingMul = 0x32,

    // ========================================================================
    // Wrapping Arithmetic (0x40-0x4F) - Modular arithmetic
    // ========================================================================
    // Operations that wrap around on overflow (modular arithmetic).
    // Format includes bit-width: `dst:reg, a:reg, b:reg, width:u8`

    /// Wrapping addition - result mod 2^width.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8`
    WrappingAdd = 0x40,

    /// Wrapping subtraction - result mod 2^width.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8`
    WrappingSub = 0x41,

    /// Wrapping multiplication - result mod 2^width.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8`
    WrappingMul = 0x42,

    /// Wrapping negation - handles MIN value correctly.
    ///
    /// Format: `dst:reg, src:reg, width:u8, signed:u8`
    WrappingNeg = 0x43,

    /// Wrapping left shift - shift amount mod width.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8`
    WrappingShl = 0x44,

    /// Wrapping right shift - shift amount mod width.
    ///
    /// Format: `dst:reg, a:reg, b:reg, width:u8`
    WrappingShr = 0x45,

    // ========================================================================
    // Bit Counting Operations (0x50-0x5F) - Leading/trailing zeros, popcount
    // ========================================================================

    /// Count leading zeros - number of 0 bits from MSB.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Returns the number of leading zero bits in the 64-bit value.
    Clz = 0x50,

    /// Count trailing zeros - number of 0 bits from LSB.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Returns the number of trailing zero bits in the 64-bit value.
    Ctz = 0x51,

    /// Population count - number of 1 bits.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Returns the number of set bits (Hamming weight) in the 64-bit value.
    Popcnt = 0x52,

    /// Byte swap - reverse byte order.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Reverses the byte order of a 64-bit value (big-endian <-> little-endian).
    Bswap = 0x53,

    /// Bit reverse - reverse all bits.
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Reverses the order of all 64 bits.
    BitReverse = 0x54,

    /// Rotate left - circular shift left.
    ///
    /// Format: `dst:reg, val:reg, amount:reg`
    ///
    /// Rotates bits left by (amount mod 64). Bits shifted out from the left
    /// wrap around to the right.
    RotateLeft = 0x55,

    /// Rotate right - circular shift right.
    ///
    /// Format: `dst:reg, val:reg, amount:reg`
    ///
    /// Rotates bits right by (amount mod 64). Bits shifted out from the right
    /// wrap around to the left.
    RotateRight = 0x56,

    // ========================================================================
    // Binary Float Operations (0x60-0x6F) - Two-argument float functions
    // ========================================================================
    // Standard binary floating-point operations that require two operands.
    // These are float-only operations (no integer dispatch).

    /// atan2(y, x) - Two-argument arctangent.
    ///
    /// Format: `dst:reg, y:reg, x:reg`
    ///
    /// Computes the angle in radians between the positive x-axis and the point (x, y).
    /// Returns a value in the range [-π, π].
    Atan2 = 0x60,

    /// hypot(x, y) - Hypotenuse calculation.
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    ///
    /// Computes sqrt(x² + y²) without intermediate overflow or underflow.
    /// More numerically stable than computing sqrt(x*x + y*y) directly.
    Hypot = 0x61,

    /// copysign(magnitude, sign) - Copy sign of a number.
    ///
    /// Format: `dst:reg, mag:reg, sign:reg`
    ///
    /// Returns a value with the magnitude of `mag` and the sign of `sign`.
    Copysign = 0x62,

    /// pow(base, exp) - Raise base to exponent power.
    ///
    /// Format: `dst:reg, base:reg, exp:reg`
    ///
    /// Computes base^exp for floating-point values.
    Pow = 0x63,

    /// log(x, base) - Logarithm with arbitrary base.
    ///
    /// Format: `dst:reg, x:reg, base:reg`
    ///
    /// Computes log_base(x) = ln(x) / ln(base).
    LogBase = 0x64,

    /// fmod(x, y) - Floating-point remainder (C-style).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    ///
    /// Returns x - n*y where n = trunc(x/y).
    /// Different from % which uses floor division.
    Fmod = 0x65,

    /// remainder(x, y) - IEEE 754 remainder.
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    ///
    /// Returns x - n*y where n = round(x/y) to nearest integer.
    Remainder = 0x66,

    /// fdim(x, y) - Positive difference.
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    ///
    /// Returns max(x - y, 0). Returns x - y if x > y, otherwise 0.
    Fdim = 0x67,

    // ========================================================================
    // Type Conversions (0x70-0x7F) - Width and precision conversions
    // ========================================================================
    // Operations that change the bit width or precision of values.
    // These map directly to LLVM sext/zext/trunc/fptrunc/fpext instructions.

    /// Sign-extend integer to wider type.
    ///
    /// Format: `dst:reg, src:reg, from_bits:u8, to_bits:u8`
    ///
    /// Extends a signed integer from `from_bits` to `to_bits`, preserving sign.
    /// The high bits of the result are filled with copies of the sign bit.
    /// Maps to LLVM `sext` instruction.
    SextI = 0x70,

    /// Zero-extend integer to wider type.
    ///
    /// Format: `dst:reg, src:reg, from_bits:u8, to_bits:u8`
    ///
    /// Extends an unsigned integer from `from_bits` to `to_bits`.
    /// The high bits of the result are filled with zeros.
    /// Maps to LLVM `zext` instruction.
    ZextI = 0x71,

    /// Truncate float precision (f64 -> f32).
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Truncates a 64-bit float to 32-bit float, potentially losing precision.
    /// Maps to LLVM `fptrunc double to float` instruction.
    FptruncF = 0x72,

    /// Extend float precision (f32 -> f64).
    ///
    /// Format: `dst:reg, src:reg`
    ///
    /// Extends a 32-bit float to 64-bit float without loss of precision.
    /// Maps to LLVM `fpext float to double` instruction.
    FpextF = 0x73,

    /// Truncate integer to narrower type.
    ///
    /// Format: `dst:reg, src:reg, to_bits:u8`
    ///
    /// Truncates an integer to a narrower bit width by discarding high bits.
    /// Maps to LLVM `trunc` instruction.
    IntTrunc = 0x74,

    /// Reinterpret f32 bits as u32.
    ///
    /// Format: `dst:reg, src:reg`
    F32ToBits = 0x75,

    /// Reinterpret u32 bits as f32.
    ///
    /// Format: `dst:reg, src:reg`
    F32FromBits = 0x76,

    /// Reinterpret f64 bits as u64.
    ///
    /// Format: `dst:reg, src:reg`
    F64ToBits = 0x77,

    /// Reinterpret u64 bits as f64.
    ///
    /// Format: `dst:reg, src:reg`
    F64FromBits = 0x78,
}

impl ArithSubOpcode {
    /// Creates an arithmetic sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Checked Arithmetic (signed)
            0x00 => Some(Self::CheckedAddI),
            0x01 => Some(Self::CheckedSubI),
            0x02 => Some(Self::CheckedMulI),
            0x03 => Some(Self::CheckedDivI),
            // Checked Arithmetic (unsigned)
            0x04 => Some(Self::CheckedAddU),
            0x05 => Some(Self::CheckedSubU),
            0x06 => Some(Self::CheckedMulU),
            // Checked unary signed (closes T::MIN gap)
            0x07 => Some(Self::CheckedNeg),
            0x08 => Some(Self::CheckedAbs),
            // Overflowing Arithmetic
            0x10 => Some(Self::OverflowingAddI),
            0x11 => Some(Self::OverflowingSubI),
            0x12 => Some(Self::OverflowingMulI),
            // Polymorphic Arithmetic
            0x20 => Some(Self::PolyAdd),
            0x21 => Some(Self::PolySub),
            0x22 => Some(Self::PolyMul),
            0x23 => Some(Self::PolyDiv),
            0x24 => Some(Self::PolyNeg),
            0x25 => Some(Self::PolyRem),
            0x26 => Some(Self::PolyAbs),
            0x27 => Some(Self::PolySignum),
            0x28 => Some(Self::PolyMin),
            0x29 => Some(Self::PolyMax),
            0x2A => Some(Self::PolyClamp),
            // Saturating Arithmetic
            0x30 => Some(Self::SaturatingAdd),
            0x31 => Some(Self::SaturatingSub),
            0x32 => Some(Self::SaturatingMul),
            0x33 => Some(Self::SaturatingNeg),
            0x34 => Some(Self::SaturatingAbs),
            // Wrapping Arithmetic
            0x40 => Some(Self::WrappingAdd),
            0x41 => Some(Self::WrappingSub),
            0x42 => Some(Self::WrappingMul),
            0x43 => Some(Self::WrappingNeg),
            0x44 => Some(Self::WrappingShl),
            0x45 => Some(Self::WrappingShr),
            // Bit Counting Operations
            0x50 => Some(Self::Clz),
            0x51 => Some(Self::Ctz),
            0x52 => Some(Self::Popcnt),
            0x53 => Some(Self::Bswap),
            0x54 => Some(Self::BitReverse),
            0x55 => Some(Self::RotateLeft),
            0x56 => Some(Self::RotateRight),
            // Binary Float Operations
            0x60 => Some(Self::Atan2),
            0x61 => Some(Self::Hypot),
            0x62 => Some(Self::Copysign),
            0x63 => Some(Self::Pow),
            0x64 => Some(Self::LogBase),
            0x65 => Some(Self::Fmod),
            0x66 => Some(Self::Remainder),
            0x67 => Some(Self::Fdim),
            // Type Conversions
            0x70 => Some(Self::SextI),
            0x71 => Some(Self::ZextI),
            0x72 => Some(Self::FptruncF),
            0x73 => Some(Self::FpextF),
            0x74 => Some(Self::IntTrunc),
            // Float bit reinterpretation
            0x75 => Some(Self::F32ToBits),
            0x76 => Some(Self::F32FromBits),
            0x77 => Some(Self::F64ToBits),
            0x78 => Some(Self::F64FromBits),
            _ => None,
        }
    }

    /// Returns the byte value of this arithmetic sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this arithmetic sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::CheckedAddI => "CHECKED_ADD_I",
            Self::CheckedSubI => "CHECKED_SUB_I",
            Self::CheckedMulI => "CHECKED_MUL_I",
            Self::CheckedDivI => "CHECKED_DIV_I",
            Self::CheckedAddU => "CHECKED_ADD_U",
            Self::CheckedSubU => "CHECKED_SUB_U",
            Self::CheckedMulU => "CHECKED_MUL_U",
            Self::CheckedNeg => "CHECKED_NEG",
            Self::CheckedAbs => "CHECKED_ABS",
            Self::OverflowingAddI => "OVERFLOWING_ADD_I",
            Self::OverflowingSubI => "OVERFLOWING_SUB_I",
            Self::OverflowingMulI => "OVERFLOWING_MUL_I",
            Self::PolyAdd => "POLY_ADD",
            Self::PolySub => "POLY_SUB",
            Self::PolyMul => "POLY_MUL",
            Self::PolyDiv => "POLY_DIV",
            Self::PolyNeg => "POLY_NEG",
            Self::PolyRem => "POLY_REM",
            Self::PolyAbs => "POLY_ABS",
            Self::PolySignum => "POLY_SIGNUM",
            Self::PolyMin => "POLY_MIN",
            Self::PolyMax => "POLY_MAX",
            Self::PolyClamp => "POLY_CLAMP",
            Self::SaturatingAdd => "SATURATING_ADD",
            Self::SaturatingSub => "SATURATING_SUB",
            Self::SaturatingMul => "SATURATING_MUL",
            Self::SaturatingNeg => "SATURATING_NEG",
            Self::SaturatingAbs => "SATURATING_ABS",
            Self::WrappingAdd => "WRAPPING_ADD",
            Self::WrappingSub => "WRAPPING_SUB",
            Self::WrappingMul => "WRAPPING_MUL",
            Self::WrappingNeg => "WRAPPING_NEG",
            Self::WrappingShl => "WRAPPING_SHL",
            Self::WrappingShr => "WRAPPING_SHR",
            Self::Clz => "CLZ",
            Self::Ctz => "CTZ",
            Self::Popcnt => "POPCNT",
            Self::Bswap => "BSWAP",
            Self::BitReverse => "BIT_REVERSE",
            Self::RotateLeft => "ROTATE_LEFT",
            Self::RotateRight => "ROTATE_RIGHT",
            Self::Atan2 => "ATAN2",
            Self::Hypot => "HYPOT",
            Self::Copysign => "COPYSIGN",
            Self::Pow => "POW",
            Self::LogBase => "LOG_BASE",
            Self::Fmod => "FMOD",
            Self::Remainder => "REMAINDER",
            Self::Fdim => "FDIM",
            Self::SextI => "SEXT_I",
            Self::ZextI => "ZEXT_I",
            Self::FptruncF => "FPTRUNC_F",
            Self::FpextF => "FPEXT_F",
            Self::IntTrunc => "INT_TRUNC",
            Self::F32ToBits => "F32_TO_BITS",
            Self::F32FromBits => "F32_FROM_BITS",
            Self::F64ToBits => "F64_TO_BITS",
            Self::F64FromBits => "F64_FROM_BITS",
        }
    }

    /// Returns the category name for this sub-opcode range.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "Checked Arithmetic",
            0x10..=0x1F => "Overflowing Arithmetic",
            0x20..=0x2F => "Polymorphic Arithmetic",
            0x30..=0x3F => "Saturating Arithmetic",
            0x40..=0x4F => "Wrapping Arithmetic",
            0x50..=0x5F => "Bit Counting",
            0x60..=0x6F => "Binary Float",
            0x70..=0x7F => "Type Conversions",
            _ => "Unknown",
        }
    }

    /// Returns true if this is a checked operation (returns Maybe<T>).
    pub fn is_checked(self) -> bool {
        matches!(
            self,
            Self::CheckedAddI | Self::CheckedSubI | Self::CheckedMulI | Self::CheckedDivI
                | Self::CheckedAddU | Self::CheckedSubU | Self::CheckedMulU
        )
    }

    /// Returns true if this is an overflowing operation (returns (result, flag)).
    pub fn is_overflowing(self) -> bool {
        matches!(
            self,
            Self::OverflowingAddI | Self::OverflowingSubI | Self::OverflowingMulI
        )
    }

    /// Returns true if this is a polymorphic operation (type-dispatched).
    pub fn is_polymorphic(self) -> bool {
        matches!(
            self,
            Self::PolyAdd | Self::PolySub | Self::PolyMul | Self::PolyDiv | Self::PolyNeg | Self::PolyRem
        )
    }

    /// Returns true if this is a binary float operation (float-only, two operands).
    pub fn is_binary_float(self) -> bool {
        matches!(
            self,
            Self::Atan2
                | Self::Hypot
                | Self::Copysign
                | Self::Pow
                | Self::LogBase
                | Self::Fmod
                | Self::Remainder
                | Self::Fdim
        )
    }

    /// Returns the number of source operands for this operation.
    pub fn operand_count(self) -> usize {
        match self {
            // Unary operations
            Self::PolyNeg | Self::PolyAbs | Self::PolySignum | Self::WrappingNeg => 1,
            // Unary bit counting
            Self::Clz | Self::Ctz | Self::Popcnt | Self::Bswap | Self::BitReverse => 1,
            // Ternary operations
            Self::PolyClamp => 3,
            // Binary operations (default)
            _ => 2,
        }
    }
}

// ============================================================================
// Comparison Extended Sub-Opcodes
// ============================================================================

/// Comparison extended sub-opcodes for unsigned integer comparisons.
///
/// Used with `CmpExtended` (0x4F) prefix opcode.
///
/// # Encoding
///
/// ```text
/// [0x4F] [sub_opcode:u8] [dst:reg] [a:reg] [b:reg]
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CmpSubOpcode {
    /// Unsigned less than: `dst = (a <u b)`
    LtU = 0x00,
    /// Unsigned less or equal: `dst = (a <=u b)`
    LeU = 0x01,
    /// Unsigned greater than: `dst = (a >u b)`
    GtU = 0x02,
    /// Unsigned greater or equal: `dst = (a >=u b)`
    GeU = 0x03,
}

impl CmpSubOpcode {
    /// Creates a comparison sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::LtU),
            0x01 => Some(Self::LeU),
            0x02 => Some(Self::GtU),
            0x03 => Some(Self::GeU),
            _ => None,
        }
    }

    /// Returns the byte value of this sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns a human-readable name for this sub-opcode.
    pub fn name(self) -> &'static str {
        match self {
            Self::LtU => "LT_U",
            Self::LeU => "LE_U",
            Self::GtU => "GT_U",
            Self::GeU => "GE_U",
        }
    }
}

// ============================================================================
// Math Extended Sub-Opcodes
// ============================================================================

/// Math Extended sub-opcodes for transcendental and special functions.
///
/// Used with `MathExtended` (0x29) opcode prefix. All operations are implemented
/// using native Rust methods which map directly to LLVM intrinsics for AOT compilation.
///
/// # Sub-opcode Ranges
///
/// - 0x00-0x07: Trigonometric F64 (sin, cos, tan, asin, acos, atan, atan2)
/// - 0x08-0x0F: Trigonometric F32
/// - 0x10-0x17: Hyperbolic F64 (sinh, cosh, tanh, asinh, acosh, atanh)
/// - 0x18-0x1F: Hyperbolic F32
/// - 0x20-0x28: Exponential F64 (exp, exp2, expm1, log, log2, log10, log1p, pow, powi)
/// - 0x29-0x2F: Exponential F32
/// - 0x30-0x37: Root/Power F64 (sqrt, cbrt, hypot)
/// - 0x38-0x3F: Root/Power F32
/// - 0x40-0x47: Rounding F64 (floor, ceil, round, trunc)
/// - 0x48-0x4F: Rounding F32
/// - 0x50-0x57: Special F64 (abs, copysign, fma, fmod, remainder, fdim, minnum, maxnum)
/// - 0x58-0x5F: Special F32
/// - 0x60-0x67: Classification F64 (is_nan, is_inf, is_finite)
/// - 0x68-0x6F: Classification F32
///
/// # Performance
///
/// - Interpreter: ~2ns per operation (native Rust method call)
/// - AOT (LLVM): Maps to `llvm.sin.f64`, `llvm.sqrt.f64`, etc.
/// - AOT (MLIR): Maps to `math.sin`, `math.sqrt`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MathSubOpcode {
    // ========================================================================
    // Trigonometric F64 (0x00-0x07)
    // ========================================================================
    /// sin(x) - Sine (F64).
    ///
    /// Format: `dst:reg, src:reg`
    SinF64 = 0x00,
    /// cos(x) - Cosine (F64).
    ///
    /// Format: `dst:reg, src:reg`
    CosF64 = 0x01,
    /// tan(x) - Tangent (F64).
    ///
    /// Format: `dst:reg, src:reg`
    TanF64 = 0x02,
    /// asin(x) - Arc sine (F64).
    ///
    /// Format: `dst:reg, src:reg`
    AsinF64 = 0x03,
    /// acos(x) - Arc cosine (F64).
    ///
    /// Format: `dst:reg, src:reg`
    AcosF64 = 0x04,
    /// atan(x) - Arc tangent (F64).
    ///
    /// Format: `dst:reg, src:reg`
    AtanF64 = 0x05,
    /// atan2(y, x) - Two-argument arc tangent (F64).
    ///
    /// Format: `dst:reg, y:reg, x:reg`
    Atan2F64 = 0x06,

    // ========================================================================
    // Trigonometric F32 (0x08-0x0F)
    // ========================================================================
    /// sin(x) - Sine (F32).
    SinF32 = 0x08,
    /// cos(x) - Cosine (F32).
    CosF32 = 0x09,
    /// tan(x) - Tangent (F32).
    TanF32 = 0x0A,
    /// asin(x) - Arc sine (F32).
    AsinF32 = 0x0B,
    /// acos(x) - Arc cosine (F32).
    AcosF32 = 0x0C,
    /// atan(x) - Arc tangent (F32).
    AtanF32 = 0x0D,
    /// atan2(y, x) - Two-argument arc tangent (F32).
    Atan2F32 = 0x0E,

    // ========================================================================
    // Hyperbolic F64 (0x10-0x17)
    // ========================================================================
    /// sinh(x) - Hyperbolic sine (F64).
    SinhF64 = 0x10,
    /// cosh(x) - Hyperbolic cosine (F64).
    CoshF64 = 0x11,
    /// tanh(x) - Hyperbolic tangent (F64).
    TanhF64 = 0x12,
    /// asinh(x) - Inverse hyperbolic sine (F64).
    AsinhF64 = 0x13,
    /// acosh(x) - Inverse hyperbolic cosine (F64).
    AcoshF64 = 0x14,
    /// atanh(x) - Inverse hyperbolic tangent (F64).
    AtanhF64 = 0x15,

    // ========================================================================
    // Hyperbolic F32 (0x18-0x1F)
    // ========================================================================
    /// sinh(x) - Hyperbolic sine (F32).
    SinhF32 = 0x18,
    /// cosh(x) - Hyperbolic cosine (F32).
    CoshF32 = 0x19,
    /// tanh(x) - Hyperbolic tangent (F32).
    TanhF32 = 0x1A,
    /// asinh(x) - Inverse hyperbolic sine (F32).
    AsinhF32 = 0x1B,
    /// acosh(x) - Inverse hyperbolic cosine (F32).
    AcoshF32 = 0x1C,
    /// atanh(x) - Inverse hyperbolic tangent (F32).
    AtanhF32 = 0x1D,

    // ========================================================================
    // Exponential/Logarithmic F64 (0x20-0x28)
    // ========================================================================
    /// exp(x) - e^x (F64).
    ExpF64 = 0x20,
    /// exp2(x) - 2^x (F64).
    Exp2F64 = 0x21,
    /// expm1(x) - e^x - 1 (F64). More accurate for small x.
    Expm1F64 = 0x22,
    /// log(x) - Natural logarithm (F64).
    LogF64 = 0x23,
    /// log2(x) - Base-2 logarithm (F64).
    Log2F64 = 0x24,
    /// log10(x) - Base-10 logarithm (F64).
    Log10F64 = 0x25,
    /// log1p(x) - ln(1 + x) (F64). More accurate for small x.
    Log1pF64 = 0x26,
    /// pow(base, exp) - base^exp (F64).
    ///
    /// Format: `dst:reg, base:reg, exp:reg`
    PowF64 = 0x27,
    /// powi(base, int_exp) - base^int_exp (F64, i32).
    ///
    /// Format: `dst:reg, base:reg, exp:reg`
    PowiF64 = 0x28,

    // ========================================================================
    // Exponential/Logarithmic F32 (0x29-0x2F)
    // ========================================================================
    /// exp(x) - e^x (F32).
    ExpF32 = 0x29,
    /// exp2(x) - 2^x (F32).
    Exp2F32 = 0x2A,
    /// expm1(x) - e^x - 1 (F32).
    Expm1F32 = 0x2B,
    /// log(x) - Natural logarithm (F32).
    LogF32 = 0x2C,
    /// log2(x) - Base-2 logarithm (F32).
    Log2F32 = 0x2D,
    /// log10(x) - Base-10 logarithm (F32).
    Log10F32 = 0x2E,
    /// log1p(x) - ln(1 + x) (F32).
    Log1pF32 = 0x2F,

    // ========================================================================
    // Root/Power Functions F64 (0x30-0x37)
    // ========================================================================
    /// sqrt(x) - Square root (F64).
    SqrtF64 = 0x30,
    /// cbrt(x) - Cube root (F64).
    CbrtF64 = 0x31,
    /// hypot(x, y) - sqrt(x² + y²) without overflow (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    HypotF64 = 0x32,

    // ========================================================================
    // Root/Power Functions F32 (0x38-0x3F)
    // ========================================================================
    /// sqrt(x) - Square root (F32).
    SqrtF32 = 0x38,
    /// cbrt(x) - Cube root (F32).
    CbrtF32 = 0x39,
    /// hypot(x, y) - sqrt(x² + y²) without overflow (F32).
    HypotF32 = 0x3A,
    /// pow(base, exp) - base^exp (F32).
    PowF32 = 0x3B,
    /// powi(base, int_exp) - base^int_exp (F32).
    PowiF32 = 0x3C,

    // ========================================================================
    // Rounding F64 (0x40-0x47)
    // ========================================================================
    /// floor(x) - Round toward negative infinity (F64).
    FloorF64 = 0x40,
    /// ceil(x) - Round toward positive infinity (F64).
    CeilF64 = 0x41,
    /// round(x) - Round to nearest integer (F64).
    RoundF64 = 0x42,
    /// trunc(x) - Round toward zero (F64).
    TruncF64 = 0x43,

    // ========================================================================
    // Rounding F32 (0x48-0x4F)
    // ========================================================================
    /// floor(x) - Round toward negative infinity (F32).
    FloorF32 = 0x48,
    /// ceil(x) - Round toward positive infinity (F32).
    CeilF32 = 0x49,
    /// round(x) - Round to nearest integer (F32).
    RoundF32 = 0x4A,
    /// trunc(x) - Round toward zero (F32).
    TruncF32 = 0x4B,

    // ========================================================================
    // Special Functions F64 (0x50-0x57)
    // ========================================================================
    /// abs(x) - Absolute value (F64).
    AbsF64 = 0x50,
    /// copysign(magnitude, sign) - Copy sign (F64).
    ///
    /// Format: `dst:reg, mag:reg, sign:reg`
    CopysignF64 = 0x51,
    /// fma(a, b, c) - Fused multiply-add: a*b + c (F64).
    ///
    /// Format: `dst:reg, a:reg, b:reg, c:reg`
    FmaF64 = 0x52,
    /// fmod(x, y) - Floating-point remainder (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    FmodF64 = 0x53,
    /// remainder(x, y) - IEEE 754 remainder (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    RemainderF64 = 0x54,
    /// fdim(x, y) - Positive difference: max(x-y, 0) (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    FdimF64 = 0x55,
    /// minnum(x, y) - Minimum (NaN-propagating) (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    MinnumF64 = 0x56,
    /// maxnum(x, y) - Maximum (NaN-propagating) (F64).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    MaxnumF64 = 0x57,

    // ========================================================================
    // Special Functions F32 (0x58-0x5F)
    // ========================================================================
    /// abs(x) - Absolute value (F32).
    AbsF32 = 0x58,
    /// copysign(magnitude, sign) - Copy sign (F32).
    CopysignF32 = 0x59,
    /// fma(a, b, c) - Fused multiply-add (F32).
    FmaF32 = 0x5A,
    /// minnum(x, y) - Minimum (NaN-propagating) (F32).
    MinnumF32 = 0x5B,
    /// maxnum(x, y) - Maximum (NaN-propagating) (F32).
    MaxnumF32 = 0x5C,
    /// fmod(x, y) - Floating-point remainder (F32).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    FmodF32 = 0x5D,
    /// remainder(x, y) - IEEE 754 remainder (F32).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    RemainderF32 = 0x5E,
    /// fdim(x, y) - Positive difference: max(x-y, 0) (F32).
    ///
    /// Format: `dst:reg, x:reg, y:reg`
    FdimF32 = 0x5F,

    // ========================================================================
    // Classification F64 (0x60-0x67)
    // ========================================================================
    /// is_nan(x) - Check if NaN (F64).
    IsNanF64 = 0x60,
    /// is_infinite(x) - Check if infinite (F64).
    IsInfF64 = 0x61,
    /// is_finite(x) - Check if finite (F64).
    IsFiniteF64 = 0x62,

    // ========================================================================
    // Classification F32 (0x68-0x6F)
    // ========================================================================
    /// is_nan(x) - Check if NaN (F32).
    IsNanF32 = 0x68,
    /// is_infinite(x) - Check if infinite (F32).
    IsInfF32 = 0x69,
    /// is_finite(x) - Check if finite (F32).
    IsFiniteF32 = 0x6A,
}

impl MathSubOpcode {
    /// Creates a math sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Trigonometric F64
            0x00 => Some(Self::SinF64),
            0x01 => Some(Self::CosF64),
            0x02 => Some(Self::TanF64),
            0x03 => Some(Self::AsinF64),
            0x04 => Some(Self::AcosF64),
            0x05 => Some(Self::AtanF64),
            0x06 => Some(Self::Atan2F64),
            // Trigonometric F32
            0x08 => Some(Self::SinF32),
            0x09 => Some(Self::CosF32),
            0x0A => Some(Self::TanF32),
            0x0B => Some(Self::AsinF32),
            0x0C => Some(Self::AcosF32),
            0x0D => Some(Self::AtanF32),
            0x0E => Some(Self::Atan2F32),
            // Hyperbolic F64
            0x10 => Some(Self::SinhF64),
            0x11 => Some(Self::CoshF64),
            0x12 => Some(Self::TanhF64),
            0x13 => Some(Self::AsinhF64),
            0x14 => Some(Self::AcoshF64),
            0x15 => Some(Self::AtanhF64),
            // Hyperbolic F32
            0x18 => Some(Self::SinhF32),
            0x19 => Some(Self::CoshF32),
            0x1A => Some(Self::TanhF32),
            0x1B => Some(Self::AsinhF32),
            0x1C => Some(Self::AcoshF32),
            0x1D => Some(Self::AtanhF32),
            // Exponential F64
            0x20 => Some(Self::ExpF64),
            0x21 => Some(Self::Exp2F64),
            0x22 => Some(Self::Expm1F64),
            0x23 => Some(Self::LogF64),
            0x24 => Some(Self::Log2F64),
            0x25 => Some(Self::Log10F64),
            0x26 => Some(Self::Log1pF64),
            0x27 => Some(Self::PowF64),
            0x28 => Some(Self::PowiF64),
            // Exponential F32
            0x29 => Some(Self::ExpF32),
            0x2A => Some(Self::Exp2F32),
            0x2B => Some(Self::Expm1F32),
            0x2C => Some(Self::LogF32),
            0x2D => Some(Self::Log2F32),
            0x2E => Some(Self::Log10F32),
            0x2F => Some(Self::Log1pF32),
            // Root F64
            0x30 => Some(Self::SqrtF64),
            0x31 => Some(Self::CbrtF64),
            0x32 => Some(Self::HypotF64),
            // Root F32
            0x38 => Some(Self::SqrtF32),
            0x39 => Some(Self::CbrtF32),
            0x3A => Some(Self::HypotF32),
            0x3B => Some(Self::PowF32),
            0x3C => Some(Self::PowiF32),
            // Rounding F64
            0x40 => Some(Self::FloorF64),
            0x41 => Some(Self::CeilF64),
            0x42 => Some(Self::RoundF64),
            0x43 => Some(Self::TruncF64),
            // Rounding F32
            0x48 => Some(Self::FloorF32),
            0x49 => Some(Self::CeilF32),
            0x4A => Some(Self::RoundF32),
            0x4B => Some(Self::TruncF32),
            // Special F64
            0x50 => Some(Self::AbsF64),
            0x51 => Some(Self::CopysignF64),
            0x52 => Some(Self::FmaF64),
            0x53 => Some(Self::FmodF64),
            0x54 => Some(Self::RemainderF64),
            0x55 => Some(Self::FdimF64),
            0x56 => Some(Self::MinnumF64),
            0x57 => Some(Self::MaxnumF64),
            // Special F32
            0x58 => Some(Self::AbsF32),
            0x59 => Some(Self::CopysignF32),
            0x5A => Some(Self::FmaF32),
            0x5B => Some(Self::MinnumF32),
            0x5C => Some(Self::MaxnumF32),
            0x5D => Some(Self::FmodF32),
            0x5E => Some(Self::RemainderF32),
            0x5F => Some(Self::FdimF32),
            // Classification F64
            0x60 => Some(Self::IsNanF64),
            0x61 => Some(Self::IsInfF64),
            0x62 => Some(Self::IsFiniteF64),
            // Classification F32
            0x68 => Some(Self::IsNanF32),
            0x69 => Some(Self::IsInfF32),
            0x6A => Some(Self::IsFiniteF32),
            _ => None,
        }
    }

    /// Returns the byte value of this math sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this math sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Trigonometric F64
            Self::SinF64 => "SIN_F64",
            Self::CosF64 => "COS_F64",
            Self::TanF64 => "TAN_F64",
            Self::AsinF64 => "ASIN_F64",
            Self::AcosF64 => "ACOS_F64",
            Self::AtanF64 => "ATAN_F64",
            Self::Atan2F64 => "ATAN2_F64",
            // Trigonometric F32
            Self::SinF32 => "SIN_F32",
            Self::CosF32 => "COS_F32",
            Self::TanF32 => "TAN_F32",
            Self::AsinF32 => "ASIN_F32",
            Self::AcosF32 => "ACOS_F32",
            Self::AtanF32 => "ATAN_F32",
            Self::Atan2F32 => "ATAN2_F32",
            // Hyperbolic F64
            Self::SinhF64 => "SINH_F64",
            Self::CoshF64 => "COSH_F64",
            Self::TanhF64 => "TANH_F64",
            Self::AsinhF64 => "ASINH_F64",
            Self::AcoshF64 => "ACOSH_F64",
            Self::AtanhF64 => "ATANH_F64",
            // Hyperbolic F32
            Self::SinhF32 => "SINH_F32",
            Self::CoshF32 => "COSH_F32",
            Self::TanhF32 => "TANH_F32",
            Self::AsinhF32 => "ASINH_F32",
            Self::AcoshF32 => "ACOSH_F32",
            Self::AtanhF32 => "ATANH_F32",
            // Exponential F64
            Self::ExpF64 => "EXP_F64",
            Self::Exp2F64 => "EXP2_F64",
            Self::Expm1F64 => "EXPM1_F64",
            Self::LogF64 => "LOG_F64",
            Self::Log2F64 => "LOG2_F64",
            Self::Log10F64 => "LOG10_F64",
            Self::Log1pF64 => "LOG1P_F64",
            Self::PowF64 => "POW_F64",
            Self::PowiF64 => "POWI_F64",
            // Exponential F32
            Self::ExpF32 => "EXP_F32",
            Self::Exp2F32 => "EXP2_F32",
            Self::Expm1F32 => "EXPM1_F32",
            Self::LogF32 => "LOG_F32",
            Self::Log2F32 => "LOG2_F32",
            Self::Log10F32 => "LOG10_F32",
            Self::Log1pF32 => "LOG1P_F32",
            // Root F64
            Self::SqrtF64 => "SQRT_F64",
            Self::CbrtF64 => "CBRT_F64",
            Self::HypotF64 => "HYPOT_F64",
            // Root F32
            Self::SqrtF32 => "SQRT_F32",
            Self::CbrtF32 => "CBRT_F32",
            Self::HypotF32 => "HYPOT_F32",
            Self::PowF32 => "POW_F32",
            Self::PowiF32 => "POWI_F32",
            // Rounding F64
            Self::FloorF64 => "FLOOR_F64",
            Self::CeilF64 => "CEIL_F64",
            Self::RoundF64 => "ROUND_F64",
            Self::TruncF64 => "TRUNC_F64",
            // Rounding F32
            Self::FloorF32 => "FLOOR_F32",
            Self::CeilF32 => "CEIL_F32",
            Self::RoundF32 => "ROUND_F32",
            Self::TruncF32 => "TRUNC_F32",
            // Special F64
            Self::AbsF64 => "ABS_F64",
            Self::CopysignF64 => "COPYSIGN_F64",
            Self::FmaF64 => "FMA_F64",
            Self::FmodF64 => "FMOD_F64",
            Self::RemainderF64 => "REMAINDER_F64",
            Self::FdimF64 => "FDIM_F64",
            Self::MinnumF64 => "MINNUM_F64",
            Self::MaxnumF64 => "MAXNUM_F64",
            // Special F32
            Self::AbsF32 => "ABS_F32",
            Self::CopysignF32 => "COPYSIGN_F32",
            Self::FmaF32 => "FMA_F32",
            Self::MinnumF32 => "MINNUM_F32",
            Self::MaxnumF32 => "MAXNUM_F32",
            Self::FmodF32 => "FMOD_F32",
            Self::RemainderF32 => "REMAINDER_F32",
            Self::FdimF32 => "FDIM_F32",
            // Classification F64
            Self::IsNanF64 => "IS_NAN_F64",
            Self::IsInfF64 => "IS_INF_F64",
            Self::IsFiniteF64 => "IS_FINITE_F64",
            // Classification F32
            Self::IsNanF32 => "IS_NAN_F32",
            Self::IsInfF32 => "IS_INF_F32",
            Self::IsFiniteF32 => "IS_FINITE_F32",
        }
    }

    /// Returns the category name for this sub-opcode.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "Trigonometric",
            0x10..=0x1F => "Hyperbolic",
            0x20..=0x2F => "Exponential/Logarithmic",
            0x30..=0x3F => "Root/Power",
            0x40..=0x4F => "Rounding",
            0x50..=0x5F => "Special",
            0x60..=0x6F => "Classification",
            _ => "Unknown",
        }
    }

    /// Returns true if this is an F64 operation.
    pub fn is_f64(self) -> bool {
        matches!(
            self.to_byte(),
            0x00..=0x06 | 0x10..=0x15 | 0x20..=0x28 | 0x30..=0x32
            | 0x40..=0x43 | 0x50..=0x57 | 0x60..=0x62
        )
    }

    /// Returns true if this is an F32 operation.
    pub fn is_f32(self) -> bool {
        !self.is_f64()
    }

    /// Returns the number of source operands for this operation.
    pub fn operand_count(self) -> usize {
        match self {
            // Binary operations (two operands)
            Self::Atan2F64 | Self::Atan2F32 => 2,
            Self::PowF64 | Self::PowiF64 | Self::PowF32 | Self::PowiF32 => 2,
            Self::HypotF64 | Self::HypotF32 => 2,
            Self::CopysignF64 | Self::CopysignF32 => 2,
            Self::FmodF64 | Self::RemainderF64 | Self::FdimF64
            | Self::FmodF32 | Self::RemainderF32 | Self::FdimF32 => 2,
            Self::MinnumF64 | Self::MaxnumF64 | Self::MinnumF32 | Self::MaxnumF32 => 2,
            // Ternary operations (three operands)
            Self::FmaF64 | Self::FmaF32 => 3,
            // Unary operations (one operand) - default
            _ => 1,
        }
    }

    /// Returns the LLVM intrinsic name for this operation.
    pub fn llvm_intrinsic(self) -> &'static str {
        match self {
            Self::SinF64 => "llvm.sin.f64",
            Self::CosF64 => "llvm.cos.f64",
            Self::TanF64 => "llvm.tan.f64",
            Self::AsinF64 => "llvm.asin.f64",
            Self::AcosF64 => "llvm.acos.f64",
            Self::AtanF64 => "llvm.atan.f64",
            Self::Atan2F64 => "llvm.atan2.f64",
            Self::SinhF64 => "llvm.sinh.f64",
            Self::CoshF64 => "llvm.cosh.f64",
            Self::TanhF64 => "llvm.tanh.f64",
            Self::AsinhF64 => "llvm.asinh.f64",
            Self::AcoshF64 => "llvm.acosh.f64",
            Self::AtanhF64 => "llvm.atanh.f64",
            Self::ExpF64 => "llvm.exp.f64",
            Self::Exp2F64 => "llvm.exp2.f64",
            Self::Expm1F64 => "llvm.expm1.f64",
            Self::LogF64 => "llvm.log.f64",
            Self::Log2F64 => "llvm.log2.f64",
            Self::Log10F64 => "llvm.log10.f64",
            Self::Log1pF64 => "llvm.log1p.f64",
            Self::PowF64 => "llvm.pow.f64",
            Self::PowiF64 => "llvm.powi.f64.i32",
            Self::SqrtF64 => "llvm.sqrt.f64",
            Self::CbrtF64 => "llvm.cbrt.f64",
            Self::HypotF64 => "llvm.hypot.f64",
            Self::FloorF64 => "llvm.floor.f64",
            Self::CeilF64 => "llvm.ceil.f64",
            Self::RoundF64 => "llvm.round.f64",
            Self::TruncF64 => "llvm.trunc.f64",
            Self::AbsF64 => "llvm.fabs.f64",
            Self::CopysignF64 => "llvm.copysign.f64",
            Self::FmaF64 => "llvm.fma.f64",
            Self::FmodF64 => "llvm.fmod.f64",
            Self::RemainderF64 => "llvm.remainder.f64",
            Self::FdimF64 => "llvm.fdim.f64",
            Self::MinnumF64 => "llvm.minnum.f64",
            Self::MaxnumF64 => "llvm.maxnum.f64",
            Self::IsNanF64 | Self::IsInfF64 | Self::IsFiniteF64 => "llvm.is.fpclass.f64",
            // F32 variants
            Self::SinF32 => "llvm.sin.f32",
            Self::CosF32 => "llvm.cos.f32",
            Self::TanF32 => "llvm.tan.f32",
            Self::AsinF32 => "llvm.asin.f32",
            Self::AcosF32 => "llvm.acos.f32",
            Self::AtanF32 => "llvm.atan.f32",
            Self::Atan2F32 => "llvm.atan2.f32",
            Self::SinhF32 => "llvm.sinh.f32",
            Self::CoshF32 => "llvm.cosh.f32",
            Self::TanhF32 => "llvm.tanh.f32",
            Self::AsinhF32 => "llvm.asinh.f32",
            Self::AcoshF32 => "llvm.acosh.f32",
            Self::AtanhF32 => "llvm.atanh.f32",
            Self::ExpF32 => "llvm.exp.f32",
            Self::Exp2F32 => "llvm.exp2.f32",
            Self::Expm1F32 => "llvm.expm1.f32",
            Self::LogF32 => "llvm.log.f32",
            Self::Log2F32 => "llvm.log2.f32",
            Self::Log10F32 => "llvm.log10.f32",
            Self::Log1pF32 => "llvm.log1p.f32",
            Self::SqrtF32 => "llvm.sqrt.f32",
            Self::CbrtF32 => "llvm.cbrt.f32",
            Self::HypotF32 => "llvm.hypot.f32",
            Self::PowF32 => "llvm.pow.f32",
            Self::PowiF32 => "llvm.powi.f32.i32",
            Self::FloorF32 => "llvm.floor.f32",
            Self::CeilF32 => "llvm.ceil.f32",
            Self::RoundF32 => "llvm.round.f32",
            Self::TruncF32 => "llvm.trunc.f32",
            Self::AbsF32 => "llvm.fabs.f32",
            Self::CopysignF32 => "llvm.copysign.f32",
            Self::FmaF32 => "llvm.fma.f32",
            Self::MinnumF32 => "llvm.minnum.f32",
            Self::MaxnumF32 => "llvm.maxnum.f32",
            Self::FmodF32 => "llvm.fmod.f32",
            Self::RemainderF32 => "llvm.remainder.f32",
            Self::FdimF32 => "llvm.fdim.f32",
            Self::IsNanF32 | Self::IsInfF32 | Self::IsFiniteF32 => "llvm.is.fpclass.f32",
        }
    }

    /// Returns the MLIR operation name for this operation (if available).
    pub fn mlir_op(self) -> Option<&'static str> {
        match self {
            Self::SinF64 | Self::SinF32 => Some("math.sin"),
            Self::CosF64 | Self::CosF32 => Some("math.cos"),
            Self::TanF64 | Self::TanF32 => Some("math.tan"),
            Self::AsinF64 | Self::AsinF32 => Some("math.asin"),
            Self::AcosF64 | Self::AcosF32 => Some("math.acos"),
            Self::AtanF64 | Self::AtanF32 => Some("math.atan"),
            Self::Atan2F64 | Self::Atan2F32 => Some("math.atan2"),
            Self::TanhF64 | Self::TanhF32 => Some("math.tanh"),
            Self::ExpF64 | Self::ExpF32 => Some("math.exp"),
            Self::Exp2F64 | Self::Exp2F32 => Some("math.exp2"),
            Self::Expm1F64 | Self::Expm1F32 => Some("math.expm1"),
            Self::LogF64 | Self::LogF32 => Some("math.log"),
            Self::Log2F64 | Self::Log2F32 => Some("math.log2"),
            Self::Log10F64 | Self::Log10F32 => Some("math.log10"),
            Self::Log1pF64 | Self::Log1pF32 => Some("math.log1p"),
            Self::PowF64 | Self::PowF32 => Some("math.powf"),
            Self::PowiF64 | Self::PowiF32 => Some("math.ipowi"),
            Self::SqrtF64 | Self::SqrtF32 => Some("math.sqrt"),
            Self::CbrtF64 | Self::CbrtF32 => Some("math.cbrt"),
            Self::FloorF64 | Self::FloorF32 => Some("math.floor"),
            Self::CeilF64 | Self::CeilF32 => Some("math.ceil"),
            Self::RoundF64 | Self::RoundF32 => Some("math.round"),
            Self::TruncF64 | Self::TruncF32 => Some("math.trunc"),
            Self::AbsF64 | Self::AbsF32 => Some("math.absf"),
            Self::CopysignF64 | Self::CopysignF32 => Some("math.copysign"),
            Self::FmaF64 | Self::FmaF32 => Some("math.fma"),
            // Operations without direct MLIR math dialect support use LLVM intrinsics
            _ => None,
        }
    }
}

// ============================================================================
// SIMD Extended Sub-Opcodes
// ============================================================================

/// SIMD Extended sub-opcodes for use with `SimdExtended` (0x2A) prefix.
///
/// Platform-agnostic SIMD operations that lower to:
/// - x86: AVX2/AVX-512 intrinsics
/// - ARM: NEON intrinsics
/// - MLIR: vector dialect
///
/// # Encoding
///
/// ```text
/// [0x2A] [sub_opcode:u8] [operands...]
/// ```
///
/// # Vector Widths
///
/// Operations work on the widest available SIMD register (128/256/512-bit).
/// The type and width are determined by operand types at compile time.
///
/// # Example
///
/// ```text
/// // Vector addition: r0 = r1 + r2
/// SimdExtended Add r0:v4f64, r1:v4f64, r2:v4f64
///
/// // Broadcast scalar to vector
/// SimdExtended Splat r0:v4f64, r1:f64
///
/// // Horizontal sum
/// SimdExtended ReduceAdd r0:f64, r1:v4f64
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SimdSubOpcode {
    // ========================================================================
    // Vector Creation (0x00-0x0F)
    // ========================================================================
    /// Broadcast scalar to all lanes.
    ///
    /// Format: `dst:reg, src:reg`
    Splat = 0x00,
    /// Extract single lane.
    ///
    /// Format: `dst:reg, src:reg, lane:u8`
    Extract = 0x01,
    /// Insert into single lane.
    ///
    /// Format: `dst:reg, src:reg, lane:u8, val:reg`
    Insert = 0x02,
    /// Create vector from scalars.
    ///
    /// Format: `dst:reg, elements:reg_range`
    FromScalars = 0x03,

    // ========================================================================
    // Arithmetic (0x10-0x2F)
    // ========================================================================
    /// Vector addition.
    Add = 0x10,
    /// Vector subtraction.
    Sub = 0x11,
    /// Vector multiplication.
    Mul = 0x12,
    /// Vector division.
    Div = 0x13,
    /// Vector negation.
    Neg = 0x14,
    /// Vector absolute value.
    Abs = 0x15,
    /// Vector square root.
    Sqrt = 0x16,
    /// Fused multiply-add: a*b + c.
    Fma = 0x17,
    /// Element-wise minimum.
    Min = 0x18,
    /// Element-wise maximum.
    Max = 0x19,
    /// Vector remainder (fmod).
    Rem = 0x1A,
    /// Reciprocal (1/x).
    Recip = 0x1B,
    /// Reciprocal square root (1/sqrt(x)).
    Rsqrt = 0x1C,

    // ========================================================================
    // Reductions (0x30-0x3F)
    // ========================================================================
    /// Horizontal sum.
    ReduceAdd = 0x30,
    /// Horizontal product.
    ReduceMul = 0x31,
    /// Horizontal minimum.
    ReduceMin = 0x32,
    /// Horizontal maximum.
    ReduceMax = 0x33,
    /// Horizontal AND.
    ReduceAnd = 0x34,
    /// Horizontal OR.
    ReduceOr = 0x35,
    /// Horizontal XOR.
    ReduceXor = 0x36,

    // ========================================================================
    // Comparisons (0x40-0x4F)
    // ========================================================================
    /// Element-wise equality.
    CmpEq = 0x40,
    /// Element-wise not-equal.
    CmpNe = 0x41,
    /// Element-wise less-than.
    CmpLt = 0x42,
    /// Element-wise less-or-equal.
    CmpLe = 0x43,
    /// Element-wise greater-than.
    CmpGt = 0x44,
    /// Element-wise greater-or-equal.
    CmpGe = 0x45,
    /// Blend based on mask.
    Select = 0x46,

    // ========================================================================
    // Memory (0x50-0x5F)
    // ========================================================================
    /// Load aligned vector.
    LoadAligned = 0x50,
    /// Load unaligned vector.
    LoadUnaligned = 0x51,
    /// Store aligned vector.
    StoreAligned = 0x52,
    /// Store unaligned vector.
    StoreUnaligned = 0x53,
    /// Masked load.
    MaskedLoad = 0x54,
    /// Masked store.
    MaskedStore = 0x55,
    /// Gather (indexed load).
    Gather = 0x56,
    /// Scatter (indexed store).
    Scatter = 0x57,

    // ========================================================================
    // Shuffle/Permute (0x60-0x6F)
    // ========================================================================
    /// Shuffle elements within vector.
    Shuffle = 0x60,
    /// Permute elements across vectors.
    Permute = 0x61,
    /// Reverse element order.
    Reverse = 0x62,
    /// Rotate elements.
    Rotate = 0x63,
    /// Interleave two vectors (low).
    InterleaveLow = 0x64,
    /// Interleave two vectors (high).
    InterleaveHigh = 0x65,
    /// Concatenate vectors.
    Concat = 0x66,

    // ========================================================================
    // Bitwise (0x70-0x7F)
    // ========================================================================
    /// Bitwise AND.
    BitwiseAnd = 0x70,
    /// Bitwise OR.
    BitwiseOr = 0x71,
    /// Bitwise XOR.
    BitwiseXor = 0x72,
    /// Bitwise NOT.
    BitwiseNot = 0x73,
    /// Shift left (each element).
    ShiftLeft = 0x74,
    /// Shift right (each element).
    ShiftRight = 0x75,
    /// Arithmetic shift right.
    ShiftRightArith = 0x76,
    /// And-not: a & ~b.
    AndNot = 0x77,

    // ========================================================================
    // Mask Operations (0x80-0x8F)
    // ========================================================================
    /// All lanes true.
    MaskAll = 0x80,
    /// No lanes true.
    MaskNone = 0x81,
    /// Any lane true.
    MaskAny = 0x82,
    /// Count true lanes.
    MaskCount = 0x83,
    /// First true lane index.
    MaskFirstTrue = 0x84,
    /// Compress (pack true lanes).
    Compress = 0x85,
    /// Expand (unpack with mask).
    Expand = 0x86,

    // ========================================================================
    // Type Conversion (0x90-0x9F)
    // ========================================================================
    /// Convert to different element type.
    Cast = 0x90,
    /// Convert f32 to f64 (widening).
    ConvertF32ToF64 = 0x91,
    /// Convert f64 to f32 (narrowing).
    ConvertF64ToF32 = 0x92,
    /// Convert int to float.
    ConvertIntToFloat = 0x93,
    /// Convert float to int.
    ConvertFloatToInt = 0x94,
    /// Reinterpret bits as different type.
    Bitcast = 0x95,
}

impl SimdSubOpcode {
    /// Creates a SIMD sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Vector Creation
            0x00 => Some(Self::Splat),
            0x01 => Some(Self::Extract),
            0x02 => Some(Self::Insert),
            0x03 => Some(Self::FromScalars),
            // Arithmetic
            0x10 => Some(Self::Add),
            0x11 => Some(Self::Sub),
            0x12 => Some(Self::Mul),
            0x13 => Some(Self::Div),
            0x14 => Some(Self::Neg),
            0x15 => Some(Self::Abs),
            0x16 => Some(Self::Sqrt),
            0x17 => Some(Self::Fma),
            0x18 => Some(Self::Min),
            0x19 => Some(Self::Max),
            0x1A => Some(Self::Rem),
            0x1B => Some(Self::Recip),
            0x1C => Some(Self::Rsqrt),
            // Reductions
            0x30 => Some(Self::ReduceAdd),
            0x31 => Some(Self::ReduceMul),
            0x32 => Some(Self::ReduceMin),
            0x33 => Some(Self::ReduceMax),
            0x34 => Some(Self::ReduceAnd),
            0x35 => Some(Self::ReduceOr),
            0x36 => Some(Self::ReduceXor),
            // Comparisons
            0x40 => Some(Self::CmpEq),
            0x41 => Some(Self::CmpNe),
            0x42 => Some(Self::CmpLt),
            0x43 => Some(Self::CmpLe),
            0x44 => Some(Self::CmpGt),
            0x45 => Some(Self::CmpGe),
            0x46 => Some(Self::Select),
            // Memory
            0x50 => Some(Self::LoadAligned),
            0x51 => Some(Self::LoadUnaligned),
            0x52 => Some(Self::StoreAligned),
            0x53 => Some(Self::StoreUnaligned),
            0x54 => Some(Self::MaskedLoad),
            0x55 => Some(Self::MaskedStore),
            0x56 => Some(Self::Gather),
            0x57 => Some(Self::Scatter),
            // Shuffle/Permute
            0x60 => Some(Self::Shuffle),
            0x61 => Some(Self::Permute),
            0x62 => Some(Self::Reverse),
            0x63 => Some(Self::Rotate),
            0x64 => Some(Self::InterleaveLow),
            0x65 => Some(Self::InterleaveHigh),
            0x66 => Some(Self::Concat),
            // Bitwise
            0x70 => Some(Self::BitwiseAnd),
            0x71 => Some(Self::BitwiseOr),
            0x72 => Some(Self::BitwiseXor),
            0x73 => Some(Self::BitwiseNot),
            0x74 => Some(Self::ShiftLeft),
            0x75 => Some(Self::ShiftRight),
            0x76 => Some(Self::ShiftRightArith),
            0x77 => Some(Self::AndNot),
            // Mask Operations
            0x80 => Some(Self::MaskAll),
            0x81 => Some(Self::MaskNone),
            0x82 => Some(Self::MaskAny),
            0x83 => Some(Self::MaskCount),
            0x84 => Some(Self::MaskFirstTrue),
            0x85 => Some(Self::Compress),
            0x86 => Some(Self::Expand),
            // Type Conversion
            0x90 => Some(Self::Cast),
            0x91 => Some(Self::ConvertF32ToF64),
            0x92 => Some(Self::ConvertF64ToF32),
            0x93 => Some(Self::ConvertIntToFloat),
            0x94 => Some(Self::ConvertFloatToInt),
            0x95 => Some(Self::Bitcast),
            _ => None,
        }
    }

    /// Returns the byte value of this SIMD sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this SIMD sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Splat => "SPLAT",
            Self::Extract => "EXTRACT",
            Self::Insert => "INSERT",
            Self::FromScalars => "FROM_SCALARS",
            Self::Add => "ADD",
            Self::Sub => "SUB",
            Self::Mul => "MUL",
            Self::Div => "DIV",
            Self::Neg => "NEG",
            Self::Abs => "ABS",
            Self::Sqrt => "SQRT",
            Self::Fma => "FMA",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::Rem => "REM",
            Self::Recip => "RECIP",
            Self::Rsqrt => "RSQRT",
            Self::ReduceAdd => "REDUCE_ADD",
            Self::ReduceMul => "REDUCE_MUL",
            Self::ReduceMin => "REDUCE_MIN",
            Self::ReduceMax => "REDUCE_MAX",
            Self::ReduceAnd => "REDUCE_AND",
            Self::ReduceOr => "REDUCE_OR",
            Self::ReduceXor => "REDUCE_XOR",
            Self::CmpEq => "CMP_EQ",
            Self::CmpNe => "CMP_NE",
            Self::CmpLt => "CMP_LT",
            Self::CmpLe => "CMP_LE",
            Self::CmpGt => "CMP_GT",
            Self::CmpGe => "CMP_GE",
            Self::Select => "SELECT",
            Self::LoadAligned => "LOAD_ALIGNED",
            Self::LoadUnaligned => "LOAD_UNALIGNED",
            Self::StoreAligned => "STORE_ALIGNED",
            Self::StoreUnaligned => "STORE_UNALIGNED",
            Self::MaskedLoad => "MASKED_LOAD",
            Self::MaskedStore => "MASKED_STORE",
            Self::Gather => "GATHER",
            Self::Scatter => "SCATTER",
            Self::Shuffle => "SHUFFLE",
            Self::Permute => "PERMUTE",
            Self::Reverse => "REVERSE",
            Self::Rotate => "ROTATE",
            Self::InterleaveLow => "INTERLEAVE_LOW",
            Self::InterleaveHigh => "INTERLEAVE_HIGH",
            Self::Concat => "CONCAT",
            Self::BitwiseAnd => "AND",
            Self::BitwiseOr => "OR",
            Self::BitwiseXor => "XOR",
            Self::BitwiseNot => "NOT",
            Self::ShiftLeft => "SHL",
            Self::ShiftRight => "SHR",
            Self::ShiftRightArith => "SAR",
            Self::AndNot => "ANDNOT",
            Self::MaskAll => "MASK_ALL",
            Self::MaskNone => "MASK_NONE",
            Self::MaskAny => "MASK_ANY",
            Self::MaskCount => "MASK_COUNT",
            Self::MaskFirstTrue => "MASK_FIRST_TRUE",
            Self::Compress => "COMPRESS",
            Self::Expand => "EXPAND",
            Self::Cast => "CAST",
            Self::ConvertF32ToF64 => "CVTF32_F64",
            Self::ConvertF64ToF32 => "CVTF64_F32",
            Self::ConvertIntToFloat => "CVTI_F",
            Self::ConvertFloatToInt => "CVTF_I",
            Self::Bitcast => "BITCAST",
        }
    }

    /// Returns the category name for this sub-opcode.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "Vector Creation",
            0x10..=0x2F => "Arithmetic",
            0x30..=0x3F => "Reduction",
            0x40..=0x4F => "Comparison",
            0x50..=0x5F => "Memory",
            0x60..=0x6F => "Shuffle/Permute",
            0x70..=0x7F => "Bitwise",
            0x80..=0x8F => "Mask",
            0x90..=0x9F => "Type Conversion",
            _ => "Unknown",
        }
    }

    /// Returns the number of source operands for this operation.
    pub fn operand_count(self) -> usize {
        match self {
            // Nullary (0 operands)
            Self::MaskAll | Self::MaskNone => 0,
            // Unary (1 operand)
            Self::Neg | Self::Abs | Self::Sqrt | Self::Recip | Self::Rsqrt
            | Self::BitwiseNot | Self::Reverse
            | Self::ReduceAdd | Self::ReduceMul | Self::ReduceMin | Self::ReduceMax
            | Self::ReduceAnd | Self::ReduceOr | Self::ReduceXor
            | Self::MaskAny | Self::MaskCount | Self::MaskFirstTrue
            | Self::Cast | Self::ConvertF32ToF64 | Self::ConvertF64ToF32
            | Self::ConvertIntToFloat | Self::ConvertFloatToInt | Self::Bitcast
            | Self::Splat | Self::LoadAligned | Self::LoadUnaligned => 1,
            // Binary (2 operands)
            Self::Add | Self::Sub | Self::Mul | Self::Div | Self::Min | Self::Max | Self::Rem
            | Self::CmpEq | Self::CmpNe | Self::CmpLt | Self::CmpLe | Self::CmpGt | Self::CmpGe
            | Self::BitwiseAnd | Self::BitwiseOr | Self::BitwiseXor | Self::AndNot
            | Self::ShiftLeft | Self::ShiftRight | Self::ShiftRightArith
            | Self::Shuffle | Self::Permute | Self::InterleaveLow | Self::InterleaveHigh
            | Self::StoreAligned | Self::StoreUnaligned | Self::Extract
            | Self::Compress | Self::Expand | Self::Concat | Self::Rotate => 2,
            // Ternary (3 operands)
            Self::Fma | Self::Select | Self::Insert | Self::MaskedLoad | Self::MaskedStore => 3,
            // Special
            Self::Gather | Self::Scatter => 3,
            Self::FromScalars => 1, // Takes a reg_range
        }
    }

    /// Returns the LLVM intrinsic name for this operation, if available.
    pub fn llvm_intrinsic(self) -> Option<&'static str> {
        match self {
            Self::Sqrt => Some("llvm.sqrt"),
            Self::Fma => Some("llvm.fma"),
            Self::Abs => Some("llvm.fabs"),
            Self::Min => Some("llvm.minnum"),
            Self::Max => Some("llvm.maxnum"),
            Self::ReduceAdd => Some("llvm.vector.reduce.fadd"),
            Self::ReduceMul => Some("llvm.vector.reduce.fmul"),
            Self::ReduceMin => Some("llvm.vector.reduce.fmin"),
            Self::ReduceMax => Some("llvm.vector.reduce.fmax"),
            Self::ReduceAnd => Some("llvm.vector.reduce.and"),
            Self::ReduceOr => Some("llvm.vector.reduce.or"),
            Self::ReduceXor => Some("llvm.vector.reduce.xor"),
            Self::MaskedLoad => Some("llvm.masked.load"),
            Self::MaskedStore => Some("llvm.masked.store"),
            Self::Gather => Some("llvm.masked.gather"),
            Self::Scatter => Some("llvm.masked.scatter"),
            Self::Compress => Some("llvm.masked.compressstore"),
            Self::Expand => Some("llvm.masked.expandload"),
            _ => None,
        }
    }

    /// Returns the MLIR operation name for this operation (if available).
    pub fn mlir_op(self) -> Option<&'static str> {
        match self {
            Self::Splat => Some("vector.splat"),
            Self::Extract => Some("vector.extract"),
            Self::Insert => Some("vector.insert"),
            Self::Add => Some("arith.addf"),
            Self::Sub => Some("arith.subf"),
            Self::Mul => Some("arith.mulf"),
            Self::Div => Some("arith.divf"),
            Self::Neg => Some("arith.negf"),
            Self::Fma => Some("math.fma"),
            Self::Min => Some("arith.minimumf"),
            Self::Max => Some("arith.maximumf"),
            Self::CmpEq | Self::CmpNe | Self::CmpLt | Self::CmpLe | Self::CmpGt | Self::CmpGe
                => Some("arith.cmpf"),
            Self::Select => Some("arith.select"),
            Self::LoadAligned | Self::LoadUnaligned => Some("vector.load"),
            Self::StoreAligned | Self::StoreUnaligned => Some("vector.store"),
            Self::Shuffle => Some("vector.shuffle"),
            Self::BitwiseAnd => Some("arith.andi"),
            Self::BitwiseOr => Some("arith.ori"),
            Self::BitwiseXor => Some("arith.xori"),
            Self::ShiftLeft => Some("arith.shli"),
            Self::ShiftRight => Some("arith.shrui"),
            Self::ShiftRightArith => Some("arith.shrsi"),
            Self::ReduceAdd => Some("vector.reduction"),
            Self::ReduceMul => Some("vector.reduction"),
            Self::ReduceMin => Some("vector.reduction"),
            Self::ReduceMax => Some("vector.reduction"),
            Self::Compress => Some("vector.compressstore"),
            Self::Expand => Some("vector.expandload"),
            Self::Gather => Some("vector.gather"),
            Self::Scatter => Some("vector.scatter"),
            Self::Cast => Some("arith.extf"),
            Self::Bitcast => Some("arith.bitcast"),
            _ => None,
        }
    }
}

// ============================================================================
// CBGR Extended Sub-Opcodes
// ============================================================================

/// CBGR extended sub-opcodes for use with `CbgrExtended` (0x8F) prefix.
///
/// This provides extended CBGR (Capability-Based Generational References)
/// operations for:
/// - Slice and interior references (fat pointers)
/// - Capability management (attenuate, transfer)
/// - Generation and epoch operations
/// - Advanced reference patterns
///
/// # Encoding
///
/// ```text
/// [0x8F] [sub_opcode:u8] [operands...]
/// ```
///
/// # Reference Types (as per CBGR spec)
///
/// - ThinRef<T> (16 bytes): ptr:8 + generation:4 + epoch_caps:4
/// - FatRef<T> (32 bytes): ptr:8 + generation:4 + epoch_caps:4 + metadata:8 + offset:4 + reserved:4
///
/// # Example
///
/// ```text
/// // Create slice reference from array
/// CbgrExtended RefSlice dst:r0, src:r1, start:r2, len:r3
///
/// // Create interior reference to struct field
/// CbgrExtended RefInterior dst:r4, base:r5, field_offset:16
///
/// // Attenuate capabilities (remove WRITE)
/// CbgrExtended CapAttenuate dst:r6, src:r7, mask:0x1E
///
/// // Transfer ownership
/// CbgrExtended CapTransfer dst:r8, src:r9
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CbgrSubOpcode {
    // ========================================================================
    // Slice and Interior References (0x00-0x0F)
    // ========================================================================
    /// Create slice reference from array/buffer.
    ///
    /// Creates a FatRef with length metadata from a contiguous buffer.
    /// Format: `dst:reg, src:reg, start:reg, len:reg`
    RefSlice = 0x00,

    /// Create interior reference to struct field.
    ///
    /// Creates a reference to a field within a struct, maintaining
    /// the generation and capabilities of the parent reference.
    /// Format: `dst:reg, base:reg, field_offset:u32`
    RefInterior = 0x01,

    /// Create interior reference to array element.
    ///
    /// Creates a reference to an element within an array/slice.
    /// Format: `dst:reg, base:reg, index:reg`
    RefArrayElement = 0x02,

    /// Create reference to trait object (fat pointer with vtable).
    ///
    /// Format: `dst:reg, src:reg, vtable_id:u32`
    RefTrait = 0x03,

    /// Unslice: get underlying pointer from slice reference.
    ///
    /// Extracts the raw pointer from a FatRef, checking bounds.
    /// Format: `dst:reg, slice_ref:reg`
    Unslice = 0x04,

    /// Get slice length from FatRef.
    ///
    /// Format: `dst:reg, slice_ref:reg`
    SliceLen = 0x05,

    /// Get element at index from slice (bounds-checked).
    ///
    /// Returns element value at the given index.
    /// Panics if index is out of bounds.
    /// Format: `dst:reg, slice_ref:reg, index:reg`
    SliceGet = 0x06,

    /// Get element at index from slice (unchecked).
    ///
    /// Returns element value at the given index.
    /// SAFETY: Caller must ensure index is within bounds.
    /// Format: `dst:reg, slice_ref:reg, index:reg`
    SliceGetUnchecked = 0x07,

    /// Create subslice from existing slice.
    ///
    /// Creates a new FatRef pointing to a subrange of the original slice.
    /// Format: `dst:reg, src:reg, start:reg, end:reg`
    SliceSubslice = 0x08,

    /// Split slice at index into two slices.
    ///
    /// Returns two FatRefs: [0..mid) and [mid..len).
    /// Format: `dst1:reg, dst2:reg, src:reg, mid:reg`
    SliceSplitAt = 0x09,

    /// Create a slice reference (FatRef) directly from a raw pointer + length.
    ///
    /// Unlike `RefSlice`, this does not read the source's ObjectHeader to infer
    /// element size — the raw pointer may point into the middle of an allocation
    /// (e.g. past the heap string header), so reading an ObjectHeader at that
    /// offset would be incorrect. Produces `FatRef { ptr, len, elem_size=1 }`,
    /// i.e. a byte slice. This is the lowering target for the generic stdlib
    /// `slice_from_raw_parts<T>` intrinsic, whose primary use sites are
    /// `Text.as_bytes()` and binary buffers indexed one byte at a time.
    ///
    /// Format: `dst:reg, ptr:reg, len:reg`
    RefSliceRaw = 0x0A,

    /// Create interior reference to a List<T> element by element-index.
    ///
    /// `RefArrayElement` (0x02) assumes the source is already a pointer to
    /// the first element; that contract fits raw arrays but not the List
    /// heap layout `[header][len][cap][backing_ptr]` → `[header][V0, V1, …]`.
    /// `RefListElement` walks the indirection: it reads `backing_ptr` from
    /// the List object, adds `OBJECT_HEADER_SIZE + index * size_of::<Value>()`,
    /// and stores the resulting element pointer as a `Value::from_ptr(…)`.
    /// A later `DerefMut` on that ptr writes directly into arr[index].
    ///
    /// This is the lowering target for `&mut arr[i]` / `&arr[i]` when the
    /// receiver is a List — without it, the generic RefMut path would
    /// produce a reference to a register holding a *copy* of the element,
    /// so `*r = v` would write to the copy instead of the underlying List.
    ///
    /// Format: `dst:reg, list:reg, index:reg`
    RefListElement = 0x0B,

    // ========================================================================
    // Capability Operations (0x10-0x1F)
    // ========================================================================
    /// Attenuate capabilities (remove permissions).
    ///
    /// Creates a new reference with reduced capabilities.
    /// Capabilities can only be removed, never added.
    /// Format: `dst:reg, src:reg, cap_mask:u16`
    CapAttenuate = 0x10,

    /// Transfer ownership (move semantics).
    ///
    /// Transfers ownership and invalidates the source reference.
    /// Format: `dst:reg, src:reg`
    CapTransfer = 0x11,

    /// Check if reference has specific capability.
    ///
    /// Format: `dst:reg, src:reg, cap:u8`
    /// Capabilities: READ=0x01, WRITE=0x02, ADD=0x04, REMOVE=0x08,
    ///               EXCLUSIVE=0x10, DELEGATE=0x20, ALIAS=0x40, DROP=0x80
    CapCheck = 0x12,

    /// Get current capability mask from reference.
    ///
    /// Format: `dst:reg, src:reg`
    CapGet = 0x13,

    /// Create shared reference (add ALIAS capability).
    ///
    /// Format: `dst:reg, src:reg`
    MakeShared = 0x14,

    /// Create exclusive reference (ensure EXCLUSIVE).
    ///
    /// Fails if reference is currently aliased.
    /// Format: `dst:reg, src:reg`
    MakeExclusive = 0x15,

    // ========================================================================
    // Generation and Epoch Operations (0x20-0x2F)
    // ========================================================================
    /// Get generation counter from reference.
    ///
    /// Format: `dst:reg, src:reg`
    GetGeneration = 0x20,

    /// Get epoch from reference.
    ///
    /// Format: `dst:reg, src:reg`
    GetEpoch = 0x21,

    /// Validate reference against current epoch.
    ///
    /// Returns true if reference is valid in current epoch.
    /// Format: `dst:reg, src:reg`
    ValidateEpoch = 0x22,

    /// Advance thread-local epoch.
    ///
    /// Invalidates all references from previous epochs.
    /// Format: `(no operands)`
    AdvanceEpoch = 0x23,

    /// Get current thread-local epoch.
    ///
    /// Format: `dst:reg`
    CurrentEpoch = 0x24,

    /// Pin reference to current epoch.
    ///
    /// Prevents automatic invalidation during epoch advance.
    /// Format: `dst:reg, src:reg`
    PinToEpoch = 0x25,

    // ========================================================================
    // Reference Conversion (0x30-0x3F)
    // ========================================================================
    /// Convert thin reference to fat reference (with metadata).
    ///
    /// Format: `dst:reg, src:reg, metadata:reg`
    ThinToFat = 0x30,

    /// Convert fat reference to thin reference (discard metadata).
    ///
    /// Format: `dst:reg, src:reg`
    FatToThin = 0x31,

    /// Create raw pointer from reference (unchecked).
    ///
    /// Bypasses CBGR validation. Use with caution.
    /// Format: `dst:reg, src:reg`
    ToRawPtr = 0x32,

    /// Create reference from raw pointer (unsafe).
    ///
    /// Requires explicit generation and capabilities.
    /// Format: `dst:reg, ptr:reg, generation:reg, caps:reg`
    FromRawPtr = 0x33,

    /// Reborrow reference with same capabilities.
    ///
    /// Creates a new reference that tracks the original.
    /// Format: `dst:reg, src:reg`
    Reborrow = 0x34,

    // ========================================================================
    // Debugging and Introspection (0x40-0x4F)
    // ========================================================================
    /// Dump reference metadata for debugging.
    ///
    /// Format: `src:reg`
    DebugRef = 0x40,

    /// Get reference tier (0=managed, 1=checked, 2=unsafe).
    ///
    /// Format: `dst:reg, src:reg`
    GetTier = 0x41,

    /// Check if reference is valid (not dangling).
    ///
    /// Format: `dst:reg, src:reg`
    IsValid = 0x42,

    /// Get reference count (for shared references).
    ///
    /// Format: `dst:reg, src:reg`
    RefCount = 0x43,

    // ========================================================================
    // CBGR Management (0x50-0x5F)
    // ========================================================================
    /// Create new generation counter.
    ///
    /// Allocates a new generation for tracking reference validity.
    /// Format: `dst:reg`
    NewGeneration = 0x50,

    /// Invalidate a reference.
    ///
    /// Marks the reference as invalid, preventing future access.
    /// Format: `src:reg`
    Invalidate = 0x51,

    /// Get epoch capabilities combined.
    ///
    /// Returns epoch and capabilities as combined value.
    /// Format: `dst:reg, src:reg`
    GetEpochCaps = 0x52,

    /// Begin CBGR bypass mode.
    ///
    /// Temporarily disables CBGR validation for performance.
    /// Use with extreme caution.
    /// Format: `(no operands)`
    BypassBegin = 0x53,

    /// End CBGR bypass mode.
    ///
    /// Re-enables CBGR validation.
    /// Format: `(no operands)`
    BypassEnd = 0x54,

    /// Get CBGR statistics.
    ///
    /// Returns statistics about CBGR operations.
    /// Format: `dst:reg`
    GetStats = 0x55,
}

impl CbgrSubOpcode {
    /// Creates a CBGR sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // Slice and Interior References
            0x00 => Some(Self::RefSlice),
            0x01 => Some(Self::RefInterior),
            0x02 => Some(Self::RefArrayElement),
            0x03 => Some(Self::RefTrait),
            0x04 => Some(Self::Unslice),
            0x05 => Some(Self::SliceLen),
            0x06 => Some(Self::SliceGet),
            0x07 => Some(Self::SliceGetUnchecked),
            0x08 => Some(Self::SliceSubslice),
            0x09 => Some(Self::SliceSplitAt),
            0x0A => Some(Self::RefSliceRaw),
            0x0B => Some(Self::RefListElement),
            // Capability Operations
            0x10 => Some(Self::CapAttenuate),
            0x11 => Some(Self::CapTransfer),
            0x12 => Some(Self::CapCheck),
            0x13 => Some(Self::CapGet),
            0x14 => Some(Self::MakeShared),
            0x15 => Some(Self::MakeExclusive),
            // Generation and Epoch Operations
            0x20 => Some(Self::GetGeneration),
            0x21 => Some(Self::GetEpoch),
            0x22 => Some(Self::ValidateEpoch),
            0x23 => Some(Self::AdvanceEpoch),
            0x24 => Some(Self::CurrentEpoch),
            0x25 => Some(Self::PinToEpoch),
            // Reference Conversion
            0x30 => Some(Self::ThinToFat),
            0x31 => Some(Self::FatToThin),
            0x32 => Some(Self::ToRawPtr),
            0x33 => Some(Self::FromRawPtr),
            0x34 => Some(Self::Reborrow),
            // Debugging and Introspection
            0x40 => Some(Self::DebugRef),
            0x41 => Some(Self::GetTier),
            0x42 => Some(Self::IsValid),
            0x43 => Some(Self::RefCount),
            // CBGR Management
            0x50 => Some(Self::NewGeneration),
            0x51 => Some(Self::Invalidate),
            0x52 => Some(Self::GetEpochCaps),
            0x53 => Some(Self::BypassBegin),
            0x54 => Some(Self::BypassEnd),
            0x55 => Some(Self::GetStats),
            _ => None,
        }
    }

    /// Returns the byte value of this CBGR sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this CBGR sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::RefSlice => "CBGR_REF_SLICE",
            Self::RefInterior => "CBGR_REF_INTERIOR",
            Self::RefArrayElement => "CBGR_REF_ARRAY_ELEM",
            Self::RefTrait => "CBGR_REF_TRAIT",
            Self::Unslice => "CBGR_UNSLICE",
            Self::SliceLen => "CBGR_SLICE_LEN",
            Self::SliceGet => "CBGR_SLICE_GET",
            Self::SliceGetUnchecked => "CBGR_SLICE_GET_UNCHECKED",
            Self::SliceSubslice => "CBGR_SLICE_SUBSLICE",
            Self::SliceSplitAt => "CBGR_SLICE_SPLIT_AT",
            Self::RefSliceRaw => "CBGR_REF_SLICE_RAW",
            Self::RefListElement => "CBGR_REF_LIST_ELEM",
            Self::CapAttenuate => "CBGR_CAP_ATTENUATE",
            Self::CapTransfer => "CBGR_CAP_TRANSFER",
            Self::CapCheck => "CBGR_CAP_CHECK",
            Self::CapGet => "CBGR_CAP_GET",
            Self::MakeShared => "CBGR_MAKE_SHARED",
            Self::MakeExclusive => "CBGR_MAKE_EXCLUSIVE",
            Self::GetGeneration => "CBGR_GET_GEN",
            Self::GetEpoch => "CBGR_GET_EPOCH",
            Self::ValidateEpoch => "CBGR_VALIDATE_EPOCH",
            Self::AdvanceEpoch => "CBGR_ADVANCE_EPOCH",
            Self::CurrentEpoch => "CBGR_CURRENT_EPOCH",
            Self::PinToEpoch => "CBGR_PIN_EPOCH",
            Self::ThinToFat => "CBGR_THIN_TO_FAT",
            Self::FatToThin => "CBGR_FAT_TO_THIN",
            Self::ToRawPtr => "CBGR_TO_RAW",
            Self::FromRawPtr => "CBGR_FROM_RAW",
            Self::Reborrow => "CBGR_REBORROW",
            Self::DebugRef => "CBGR_DEBUG_REF",
            Self::GetTier => "CBGR_GET_TIER",
            Self::IsValid => "CBGR_IS_VALID",
            Self::RefCount => "CBGR_REF_COUNT",
            Self::NewGeneration => "CBGR_NEW_GEN",
            Self::Invalidate => "CBGR_INVALIDATE",
            Self::GetEpochCaps => "CBGR_GET_EPOCH_CAPS",
            Self::BypassBegin => "CBGR_BYPASS_BEGIN",
            Self::BypassEnd => "CBGR_BYPASS_END",
            Self::GetStats => "CBGR_GET_STATS",
        }
    }

    /// Returns the category name for this sub-opcode range.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "Slice/Interior References",
            0x10..=0x1F => "Capability Operations",
            0x20..=0x2F => "Generation/Epoch",
            0x30..=0x3F => "Reference Conversion",
            0x40..=0x4F => "Debug/Introspection",
            0x50..=0x5F => "CBGR Management",
            _ => "Unknown",
        }
    }

    /// Returns true if this operation creates a new reference.
    pub fn creates_reference(self) -> bool {
        matches!(
            self,
            Self::RefSlice
                | Self::RefSliceRaw
                | Self::RefInterior
                | Self::RefArrayElement
                | Self::RefListElement
                | Self::RefTrait
                | Self::ThinToFat
                | Self::FromRawPtr
                | Self::Reborrow
                | Self::MakeShared
                | Self::MakeExclusive
        )
    }

    /// Returns true if this operation modifies capabilities.
    pub fn modifies_capabilities(self) -> bool {
        matches!(
            self,
            Self::CapAttenuate | Self::CapTransfer | Self::MakeShared | Self::MakeExclusive
        )
    }

    /// Returns true if this operation is a validation check.
    pub fn is_validation(self) -> bool {
        matches!(
            self,
            Self::CapCheck | Self::ValidateEpoch | Self::IsValid
        )
    }
}

// =============================================================================
// CharSubOpcode — Character Operations (CharExtended 0x2B)
// =============================================================================

/// Character Extended sub-opcodes for character classification and conversion.
///
/// The CharExtended opcode (0x2B) provides efficient character operations with
/// ASCII fast paths (inline implementation) and Unicode fallbacks (runtime lookup).
///
/// # Performance Characteristics
///
/// | Operation      | ASCII   | Unicode |
/// |---------------|---------|---------|
/// | Classification | ~2ns    | ~20ns   |
/// | Case convert   | ~2ns    | ~50ns   |
///
/// # Example Usage
///
/// ```text
/// // ASCII fast path: is_alphabetic for ASCII char
/// CharExtended IsAlphabeticAscii r0, r1
///
/// // Unicode fallback: is_alphabetic for any char
/// CharExtended IsAlphabeticUnicode r0, r1
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CharSubOpcode {
    // ========================================================================
    // ASCII Classification (0x00-0x0F) - Inline fast path
    // ========================================================================
    /// Check if char is ASCII alphabetic (a-z, A-Z).
    ///
    /// Format: `dst:reg, src:reg`
    IsAlphabeticAscii = 0x00,

    /// Check if char is ASCII digit (0-9).
    ///
    /// Format: `dst:reg, src:reg`
    IsNumericAscii = 0x01,

    /// Check if char is ASCII alphanumeric (a-z, A-Z, 0-9).
    ///
    /// Format: `dst:reg, src:reg`
    IsAlphanumericAscii = 0x02,

    /// Check if char is ASCII whitespace (space, tab, newline, etc.).
    ///
    /// Format: `dst:reg, src:reg`
    IsWhitespaceAscii = 0x03,

    /// Check if char is ASCII control character (0x00-0x1F, 0x7F).
    ///
    /// Format: `dst:reg, src:reg`
    IsControlAscii = 0x04,

    /// Check if char is ASCII punctuation.
    ///
    /// Format: `dst:reg, src:reg`
    IsPunctuationAscii = 0x05,

    /// Check if char is ASCII graphic (visible character).
    ///
    /// Format: `dst:reg, src:reg`
    IsGraphicAscii = 0x06,

    /// Check if char is ASCII hexadecimal digit (0-9, a-f, A-F).
    ///
    /// Format: `dst:reg, src:reg`
    IsHexDigitAscii = 0x07,

    /// Check if char is ASCII lowercase (a-z).
    ///
    /// Format: `dst:reg, src:reg`
    IsLowercaseAscii = 0x08,

    /// Check if char is ASCII uppercase (A-Z).
    ///
    /// Format: `dst:reg, src:reg`
    IsUppercaseAscii = 0x09,

    /// Check if char is ASCII (0x00-0x7F).
    ///
    /// Format: `dst:reg, src:reg`
    IsAscii = 0x0A,

    // ========================================================================
    // ASCII Case Conversion (0x10-0x1F) - Inline fast path
    // ========================================================================
    /// Convert char to ASCII uppercase.
    ///
    /// Non-ASCII chars are unchanged.
    /// Format: `dst:reg, src:reg`
    ToUppercaseAscii = 0x10,

    /// Convert char to ASCII lowercase.
    ///
    /// Non-ASCII chars are unchanged.
    /// Format: `dst:reg, src:reg`
    ToLowercaseAscii = 0x11,

    /// Check if char equals its ASCII uppercase form.
    ///
    /// Format: `dst:reg, src:reg`
    EqIgnoreCaseAscii = 0x12,

    // ========================================================================
    // Unicode Classification (0x20-0x2F) - Runtime lookup
    // ========================================================================
    /// Check if char is Unicode alphabetic.
    ///
    /// Uses Unicode derived property Alphabetic.
    /// Format: `dst:reg, src:reg`
    IsAlphabeticUnicode = 0x20,

    /// Check if char is Unicode numeric (Nd, Nl, No categories).
    ///
    /// Format: `dst:reg, src:reg`
    IsNumericUnicode = 0x21,

    /// Check if char is Unicode alphanumeric.
    ///
    /// Format: `dst:reg, src:reg`
    IsAlphanumericUnicode = 0x22,

    /// Check if char is Unicode whitespace.
    ///
    /// Format: `dst:reg, src:reg`
    IsWhitespaceUnicode = 0x23,

    /// Check if char is Unicode control character.
    ///
    /// Format: `dst:reg, src:reg`
    IsControlUnicode = 0x24,

    /// Check if char is Unicode lowercase.
    ///
    /// Format: `dst:reg, src:reg`
    IsLowercaseUnicode = 0x25,

    /// Check if char is Unicode uppercase.
    ///
    /// Format: `dst:reg, src:reg`
    IsUppercaseUnicode = 0x26,

    // ========================================================================
    // Unicode Case Conversion (0x30-0x3F) - Runtime lookup
    // ========================================================================
    /// Convert char to Unicode uppercase.
    ///
    /// Returns first char of uppercase mapping.
    /// Format: `dst:reg, src:reg`
    ToUppercaseUnicode = 0x30,

    /// Convert char to Unicode lowercase.
    ///
    /// Returns first char of lowercase mapping.
    /// Format: `dst:reg, src:reg`
    ToLowercaseUnicode = 0x31,

    /// Convert char to Unicode titlecase.
    ///
    /// Returns first char of titlecase mapping.
    /// Format: `dst:reg, src:reg`
    ToTitlecaseUnicode = 0x32,

    // ========================================================================
    // Char Value Operations (0x40-0x4F)
    // ========================================================================
    /// Get Unicode code point value.
    ///
    /// Format: `dst:reg, src:reg`
    ToCodePoint = 0x40,

    /// Create char from Unicode code point.
    ///
    /// Returns None/error if invalid code point.
    /// Format: `dst:reg, src:reg`
    FromCodePoint = 0x41,

    /// Get char length in UTF-8 bytes.
    ///
    /// Format: `dst:reg, src:reg`
    LenUtf8 = 0x42,

    /// Get char length in UTF-16 code units.
    ///
    /// Format: `dst:reg, src:reg`
    LenUtf16 = 0x43,

    // ========================================================================
    // UTF-8 Encoding/Decoding (0x50-0x5F)
    // ========================================================================
    /// Encode char as UTF-8 bytes.
    ///
    /// Returns the UTF-8 byte sequence for the character.
    /// Format: `dst:reg, src:reg`
    EncodeUtf8 = 0x50,

    /// Decode UTF-8 bytes to char.
    ///
    /// Returns the character from UTF-8 byte sequence.
    /// Format: `dst:reg, src:reg`
    DecodeUtf8 = 0x51,

    /// Escape char for debug output.
    ///
    /// Returns an escaped string representation.
    /// Format: `dst:reg, src:reg`
    EscapeDebug = 0x52,

    /// Get Unicode general category.
    ///
    /// Returns the Unicode general category (Lu, Ll, Nd, etc.).
    /// Format: `dst:reg, src:reg`
    GeneralCategory = 0x53,
}

impl CharSubOpcode {
    /// Creates a Char sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            // ASCII Classification
            0x00 => Some(Self::IsAlphabeticAscii),
            0x01 => Some(Self::IsNumericAscii),
            0x02 => Some(Self::IsAlphanumericAscii),
            0x03 => Some(Self::IsWhitespaceAscii),
            0x04 => Some(Self::IsControlAscii),
            0x05 => Some(Self::IsPunctuationAscii),
            0x06 => Some(Self::IsGraphicAscii),
            0x07 => Some(Self::IsHexDigitAscii),
            0x08 => Some(Self::IsLowercaseAscii),
            0x09 => Some(Self::IsUppercaseAscii),
            0x0A => Some(Self::IsAscii),
            // ASCII Case Conversion
            0x10 => Some(Self::ToUppercaseAscii),
            0x11 => Some(Self::ToLowercaseAscii),
            0x12 => Some(Self::EqIgnoreCaseAscii),
            // Unicode Classification
            0x20 => Some(Self::IsAlphabeticUnicode),
            0x21 => Some(Self::IsNumericUnicode),
            0x22 => Some(Self::IsAlphanumericUnicode),
            0x23 => Some(Self::IsWhitespaceUnicode),
            0x24 => Some(Self::IsControlUnicode),
            0x25 => Some(Self::IsLowercaseUnicode),
            0x26 => Some(Self::IsUppercaseUnicode),
            // Unicode Case Conversion
            0x30 => Some(Self::ToUppercaseUnicode),
            0x31 => Some(Self::ToLowercaseUnicode),
            0x32 => Some(Self::ToTitlecaseUnicode),
            // Char Value Operations
            0x40 => Some(Self::ToCodePoint),
            0x41 => Some(Self::FromCodePoint),
            0x42 => Some(Self::LenUtf8),
            0x43 => Some(Self::LenUtf16),
            // UTF-8 Encoding/Decoding
            0x50 => Some(Self::EncodeUtf8),
            0x51 => Some(Self::DecodeUtf8),
            0x52 => Some(Self::EscapeDebug),
            0x53 => Some(Self::GeneralCategory),
            _ => None,
        }
    }

    /// Returns the byte value of this Char sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this Char sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // ASCII Classification
            Self::IsAlphabeticAscii => "CHAR_IS_ALPHA_ASCII",
            Self::IsNumericAscii => "CHAR_IS_NUMERIC_ASCII",
            Self::IsAlphanumericAscii => "CHAR_IS_ALNUM_ASCII",
            Self::IsWhitespaceAscii => "CHAR_IS_WS_ASCII",
            Self::IsControlAscii => "CHAR_IS_CTRL_ASCII",
            Self::IsPunctuationAscii => "CHAR_IS_PUNCT_ASCII",
            Self::IsGraphicAscii => "CHAR_IS_GRAPH_ASCII",
            Self::IsHexDigitAscii => "CHAR_IS_HEX_ASCII",
            Self::IsLowercaseAscii => "CHAR_IS_LOWER_ASCII",
            Self::IsUppercaseAscii => "CHAR_IS_UPPER_ASCII",
            Self::IsAscii => "CHAR_IS_ASCII",
            // ASCII Case Conversion
            Self::ToUppercaseAscii => "CHAR_TO_UPPER_ASCII",
            Self::ToLowercaseAscii => "CHAR_TO_LOWER_ASCII",
            Self::EqIgnoreCaseAscii => "CHAR_EQ_ICASE_ASCII",
            // Unicode Classification
            Self::IsAlphabeticUnicode => "CHAR_IS_ALPHA_UNI",
            Self::IsNumericUnicode => "CHAR_IS_NUMERIC_UNI",
            Self::IsAlphanumericUnicode => "CHAR_IS_ALNUM_UNI",
            Self::IsWhitespaceUnicode => "CHAR_IS_WS_UNI",
            Self::IsControlUnicode => "CHAR_IS_CTRL_UNI",
            Self::IsLowercaseUnicode => "CHAR_IS_LOWER_UNI",
            Self::IsUppercaseUnicode => "CHAR_IS_UPPER_UNI",
            // Unicode Case Conversion
            Self::ToUppercaseUnicode => "CHAR_TO_UPPER_UNI",
            Self::ToLowercaseUnicode => "CHAR_TO_LOWER_UNI",
            Self::ToTitlecaseUnicode => "CHAR_TO_TITLE_UNI",
            // Char Value Operations
            Self::ToCodePoint => "CHAR_TO_CODEPOINT",
            Self::FromCodePoint => "CHAR_FROM_CODEPOINT",
            Self::LenUtf8 => "CHAR_LEN_UTF8",
            Self::LenUtf16 => "CHAR_LEN_UTF16",
            // UTF-8 Encoding/Decoding
            Self::EncodeUtf8 => "CHAR_ENCODE_UTF8",
            Self::DecodeUtf8 => "CHAR_DECODE_UTF8",
            Self::EscapeDebug => "CHAR_ESCAPE_DEBUG",
            Self::GeneralCategory => "CHAR_GENERAL_CATEGORY",
        }
    }

    /// Returns the category name for this sub-opcode range.
    pub fn category(self) -> &'static str {
        match self.to_byte() {
            0x00..=0x0F => "ASCII Classification",
            0x10..=0x1F => "ASCII Case Conversion",
            0x20..=0x2F => "Unicode Classification",
            0x30..=0x3F => "Unicode Case Conversion",
            0x40..=0x4F => "Char Value Operations",
            0x50..=0x5F => "UTF-8 Encoding/Decoding",
            _ => "Unknown",
        }
    }

    /// Returns true if this is an ASCII fast path operation.
    pub fn is_ascii_fast_path(self) -> bool {
        self.to_byte() < 0x20
    }

    /// Returns true if this operation returns a boolean.
    pub fn returns_bool(self) -> bool {
        matches!(
            self,
            Self::IsAlphabeticAscii
                | Self::IsNumericAscii
                | Self::IsAlphanumericAscii
                | Self::IsWhitespaceAscii
                | Self::IsControlAscii
                | Self::IsPunctuationAscii
                | Self::IsGraphicAscii
                | Self::IsHexDigitAscii
                | Self::IsLowercaseAscii
                | Self::IsUppercaseAscii
                | Self::IsAscii
                | Self::EqIgnoreCaseAscii
                | Self::IsAlphabeticUnicode
                | Self::IsNumericUnicode
                | Self::IsAlphanumericUnicode
                | Self::IsWhitespaceUnicode
                | Self::IsControlUnicode
                | Self::IsLowercaseUnicode
                | Self::IsUppercaseUnicode
        )
    }

    /// Returns true if this operation returns a char.
    pub fn returns_char(self) -> bool {
        matches!(
            self,
            Self::ToUppercaseAscii
                | Self::ToLowercaseAscii
                | Self::ToUppercaseUnicode
                | Self::ToLowercaseUnicode
                | Self::ToTitlecaseUnicode
                | Self::FromCodePoint
        )
    }
}

// =============================================================================
// LogSubOpcode — Logging Operations (LogExtended 0xBE)
// =============================================================================

/// Logging Extended sub-opcodes for structured logging operations.
///
/// The LogExtended opcode (0xBE) provides structured logging with different
/// severity levels. Logging is inherently I/O-bound, so the runtime call
/// overhead (~50ns) is negligible compared to actual I/O operations.
///
/// # Performance Characteristics
///
/// | Operation | Overhead | Notes |
/// |-----------|----------|-------|
/// | Log call  | ~50ns    | Acceptable for low-frequency I/O |
/// | Format    | ~100ns   | String interpolation |
/// | Flush     | ~1ms+    | Actual I/O |
///
/// # Example Usage
///
/// ```text
/// // Log info message
/// LogExtended Info r0    // r0 contains message string
///
/// // Log warning with formatted message
/// LogExtended Warning r1
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum LogSubOpcode {
    /// Log at INFO level.
    ///
    /// Format: `msg:reg`
    Info = 0x00,

    /// Log at WARNING level.
    ///
    /// Format: `msg:reg`
    Warning = 0x01,

    /// Log at ERROR level.
    ///
    /// Format: `msg:reg`
    Error = 0x02,

    /// Log at DEBUG level.
    ///
    /// Format: `msg:reg`
    Debug = 0x03,

    /// Log at TRACE level (most verbose).
    ///
    /// Format: `msg:reg`
    Trace = 0x04,

    /// Structured log with key-value pairs.
    ///
    /// Format: `level:u8, msg:reg, kvs:reg` (kvs is a map/object)
    Structured = 0x10,

    /// Flush log buffer to output.
    ///
    /// Format: `(no operands)`
    Flush = 0x20,

    /// Set current log level filter.
    ///
    /// Format: `level:reg`
    SetLevel = 0x21,

    /// Get current log level filter.
    ///
    /// Format: `dst:reg`
    GetLevel = 0x22,
}

impl LogSubOpcode {
    /// Creates a Log sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Info),
            0x01 => Some(Self::Warning),
            0x02 => Some(Self::Error),
            0x03 => Some(Self::Debug),
            0x04 => Some(Self::Trace),
            0x10 => Some(Self::Structured),
            0x20 => Some(Self::Flush),
            0x21 => Some(Self::SetLevel),
            0x22 => Some(Self::GetLevel),
            _ => None,
        }
    }

    /// Returns the byte value of this Log sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this Log sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Info => "LOG_INFO",
            Self::Warning => "LOG_WARNING",
            Self::Error => "LOG_ERROR",
            Self::Debug => "LOG_DEBUG",
            Self::Trace => "LOG_TRACE",
            Self::Structured => "LOG_STRUCTURED",
            Self::Flush => "LOG_FLUSH",
            Self::SetLevel => "LOG_SET_LEVEL",
            Self::GetLevel => "LOG_GET_LEVEL",
        }
    }

    /// Returns the log level as a numeric value (lower = more severe).
    pub fn severity(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Debug => 3,
            Self::Trace => 4,
            Self::Structured | Self::Flush | Self::SetLevel | Self::GetLevel => 255,
        }
    }

    /// Returns true if this is a log level operation (not a control operation).
    pub fn is_log_level(self) -> bool {
        matches!(
            self,
            Self::Info | Self::Warning | Self::Error | Self::Debug | Self::Trace | Self::Structured
        )
    }
}

/// Meta-reflection sub-opcodes for type introspection.
///
/// Used with the MetaReflect opcode (0xBB) to provide zero-cost type introspection.
///
/// # Sub-opcode Space Allocation
///
/// - 0x00-0x0F: Type information queries
/// - 0x10-0x1F: Type relationship queries
/// - 0x20-0x2F: Reserved for future use
///
/// # Performance
///
/// - Interpreter: ~2ns per operation (direct match dispatch)
/// - AOT (LLVM): Constant-folded at compile time when possible
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MetaReflectOp {
    /// Get type ID for a value or type.
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns a unique u64 identifier for the type.
    TypeId = 0x00,

    /// Get type name as Text.
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns the fully-qualified type name.
    TypeName = 0x01,

    /// Check if type requires drop (destructor).
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns bool: true if type has non-trivial destructor.
    NeedsDrop = 0x02,

    /// Check if type is Copy.
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns bool: true if type implements Copy.
    IsCopy = 0x03,

    /// Check if type is Send (thread-safe).
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns bool: true if type can be sent between threads.
    IsSend = 0x04,

    /// Check if type is Sync (shared reference thread-safe).
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns bool: true if shared references are thread-safe.
    IsSync = 0x05,

    /// Get type's minimum alignment requirement.
    ///
    /// Format: `dst:reg, type_id:reg`
    /// Returns usize: alignment in bytes.
    MinAlign = 0x06,

    /// Get type's preferred alignment for performance.
    ///
    /// Format: `dst:reg, type_id:reg`
    /// Returns usize: preferred alignment in bytes.
    PrefAlign = 0x07,
}

impl MetaReflectOp {
    /// Creates a MetaReflect sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::TypeId),
            0x01 => Some(Self::TypeName),
            0x02 => Some(Self::NeedsDrop),
            0x03 => Some(Self::IsCopy),
            0x04 => Some(Self::IsSend),
            0x05 => Some(Self::IsSync),
            0x06 => Some(Self::MinAlign),
            0x07 => Some(Self::PrefAlign),
            _ => None,
        }
    }

    /// Returns the byte value of this MetaReflect sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this MetaReflect sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::TypeId => "META_TYPE_ID",
            Self::TypeName => "META_TYPE_NAME",
            Self::NeedsDrop => "META_NEEDS_DROP",
            Self::IsCopy => "META_IS_COPY",
            Self::IsSend => "META_IS_SEND",
            Self::IsSync => "META_IS_SYNC",
            Self::MinAlign => "META_MIN_ALIGN",
            Self::PrefAlign => "META_PREF_ALIGN",
        }
    }
}

// =============================================================================
// TextSubOpcode — Text Parsing and Conversion Operations (TextExtended 0x79)
// =============================================================================

/// Text Extended sub-opcodes for text parsing and conversion operations.
///
/// The TextExtended opcode (0x79) provides zero-cost dispatch (~2ns) for text
/// operations that would otherwise require string-based library calls (~15ns).
///
/// # Sub-opcode Space Allocation
///
/// - 0x00-0x0F: Text construction
/// - 0x10-0x1F: Parse from text
/// - 0x20-0x2F: Convert to text
/// - 0x30-0x3F: Text manipulation
///
/// # Performance
///
/// | Operation | Old (LibraryCall) | New (TextExtended) |
/// |-----------|-------------------|---------------------|
/// | Dispatch  | ~15ns             | ~2ns               |
/// | Parse Int | ~50ns             | ~40ns              |
/// | Int->Text | ~100ns            | ~90ns              |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TextSubOpcode {
    /// Create Text from static string data.
    ///
    /// Format: `dst:reg, ptr:reg, len:reg`
    /// Creates a Text value from a static UTF-8 string slice.
    FromStatic = 0x00,

    /// Parse integer from Text.
    ///
    /// Format: `dst:reg, text:reg`
    /// Returns parsed integer or error.
    ParseInt = 0x10,

    /// Parse float from Text.
    ///
    /// Format: `dst:reg, text:reg`
    /// Returns parsed float or error.
    ParseFloat = 0x11,

    /// Convert integer to Text.
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns decimal string representation.
    IntToText = 0x20,

    /// Convert float to Text.
    ///
    /// Format: `dst:reg, value:reg`
    /// Returns decimal string representation.
    FloatToText = 0x21,

    /// Get Text length in bytes.
    ///
    /// Format: `dst:reg, text:reg`
    /// Returns byte length of UTF-8 encoded text.
    ByteLen = 0x30,

    /// Get Text length in characters.
    ///
    /// Format: `dst:reg, text:reg`
    /// Returns number of Unicode scalar values.
    CharLen = 0x31,

    /// Check if Text is empty.
    ///
    /// Format: `dst:reg, text:reg`
    /// Returns true if text has zero length.
    IsEmpty = 0x32,

    /// Check if Text is valid UTF-8.
    ///
    /// Format: `dst:reg, text:reg`
    /// Always returns true for Text type (guaranteed valid).
    IsUtf8 = 0x33,

    /// Borrow the Text as a byte slice (FatRef with elem_size=1).
    ///
    /// Produces a proper slice value so that downstream `.len()` and
    /// `slice[i]` calls read the correct byte count / byte value regardless
    /// of whether the Text is NaN-boxed as a small string or heap-allocated.
    /// For small strings, a fresh heap buffer is materialised and copied
    /// into so the returned FatRef has a stable address.
    ///
    /// Format: `dst:reg, text:reg`
    AsBytes = 0x34,
}

impl TextSubOpcode {
    /// Creates a Text sub-opcode from a byte value.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::FromStatic),
            0x10 => Some(Self::ParseInt),
            0x11 => Some(Self::ParseFloat),
            0x20 => Some(Self::IntToText),
            0x21 => Some(Self::FloatToText),
            0x30 => Some(Self::ByteLen),
            0x31 => Some(Self::CharLen),
            0x32 => Some(Self::IsEmpty),
            0x33 => Some(Self::IsUtf8),
            0x34 => Some(Self::AsBytes),
            _ => None,
        }
    }

    /// Returns the byte value of this Text sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this Text sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::FromStatic => "TEXT_FROM_STATIC",
            Self::ParseInt => "TEXT_PARSE_INT",
            Self::ParseFloat => "TEXT_PARSE_FLOAT",
            Self::IntToText => "TEXT_INT_TO_TEXT",
            Self::FloatToText => "TEXT_FLOAT_TO_TEXT",
            Self::ByteLen => "TEXT_BYTE_LEN",
            Self::CharLen => "TEXT_CHAR_LEN",
            Self::IsEmpty => "TEXT_IS_EMPTY",
            Self::IsUtf8 => "TEXT_IS_UTF8",
            Self::AsBytes => "TEXT_AS_BYTES",
        }
    }

    /// Returns true if this operation returns a Text value.
    pub fn returns_text(self) -> bool {
        matches!(
            self,
            Self::FromStatic | Self::IntToText | Self::FloatToText
        )
    }

    /// Returns true if this operation parses input.
    pub fn is_parse_operation(self) -> bool {
        matches!(self, Self::ParseInt | Self::ParseFloat)
    }
}

// Compile-time assertion: Opcode must have exactly 256 variants (one per u8 value).
// If this fails, from_byte's transmute is unsound.
const _: () = {
    // The enum must be exactly 1 byte (u8-sized)
    assert!(std::mem::size_of::<Opcode>() == 1, "Opcode must be repr(u8) and 1 byte");
};

/// Sub-opcodes for the general-purpose `Opcode::Extended` (`0x1F`) instruction
/// (#167 Part A).
///
/// Each sub-op carves out one of 256 entries in the secondary opcode space.
/// `Reserved` (`0x00`) is a forward-compat anchor — encoders must never emit
/// it; decoders accept and skip the (length-prefixed) operand block.  Future
/// first-class instructions land here as they're wired through codegen /
/// interpreter / LLVM (see #167 Part B for `MakeVariantTyped` 0x01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExtendedSubOpcode {
    /// Reserved no-op — forward-compat anchor.  Always sub-op `0x00`.
    /// Encoders must not emit this; decoders must accept and skip its
    /// (length-prefixed) operand block.
    Reserved = 0x00,
}

impl ExtendedSubOpcode {
    /// Creates an extension sub-opcode from a byte, or `None` if the byte
    /// is not a known sub-op.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Reserved),
            _ => None,
        }
    }

    /// Returns the byte value of this extension sub-opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this extension sub-opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Reserved => "EXT_RESERVED",
        }
    }
}

impl Opcode {
    /// Creates an opcode from a byte value.
    ///
    /// # Safety invariant
    ///
    /// This relies on all 256 u8 values being valid `Opcode` variants (the enum
    /// covers 0x00..=0xFF). The compile-time assertion above verifies the enum
    /// is u8-sized. If a variant is ever removed without a replacement, this
    /// transmute becomes unsound and must be replaced with a match.
    pub fn from_byte(byte: u8) -> Self {
        // SAFETY: All 256 values (0x00..=0xFF) are valid Opcode variants.
        // Verified by: (1) compile-time size assertion above, (2) the enum definition
        // explicitly assigns every value from 0x00 to 0xFF.
        unsafe { std::mem::transmute(byte) }
    }

    /// Returns the byte value of this opcode.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Returns the mnemonic string for this opcode.
    pub fn mnemonic(self) -> &'static str {
        match self {
            // Data Movement (0x00-0x0F)
            Opcode::Mov => "MOV",
            Opcode::LoadK => "LOAD_K",
            Opcode::LoadI => "LOAD_I",
            Opcode::LoadF => "LOAD_F",
            Opcode::LoadTrue => "LOAD_TRUE",
            Opcode::LoadFalse => "LOAD_FALSE",
            Opcode::LoadUnit => "LOAD_UNIT",
            Opcode::LoadT => "LOAD_T",
            Opcode::LoadSmallI => "LOAD_SI",
            Opcode::LoadNil => "LOAD_NIL",
            Opcode::Nop => "NOP",
            Opcode::CvtIF => "CVT_IF",
            Opcode::CvtFI => "CVT_FI",
            Opcode::CvtIC => "CVT_IC",
            Opcode::CvtCI => "CVT_CI",
            Opcode::CvtBI => "CVT_BI",
            // Integer Arithmetic (0x10-0x1F)
            Opcode::AddI => "ADD_I",
            Opcode::SubI => "SUB_I",
            Opcode::MulI => "MUL_I",
            Opcode::DivI => "DIV_I",
            Opcode::ModI => "MOD_I",
            Opcode::NegI => "NEG_I",
            Opcode::AbsI => "ABS_I",
            Opcode::PowI => "POW_I",
            Opcode::Inc => "INC",
            Opcode::Dec => "DEC",
            Opcode::CvtToI => "CVT_TO_I",
            // Float Arithmetic (0x20-0x2F)
            Opcode::AddF => "ADD_F",
            Opcode::SubF => "SUB_F",
            Opcode::MulF => "MUL_F",
            Opcode::DivF => "DIV_F",
            Opcode::ModF => "MOD_F",
            Opcode::NegF => "NEG_F",
            Opcode::AbsF => "ABS_F",
            Opcode::PowF => "POW_F",
            Opcode::CvtToF => "CVT_TO_F",
            // Bitwise + Generic Arithmetic (0x30-0x3F)
            Opcode::Band => "BAND",
            Opcode::Bor => "BOR",
            Opcode::Bxor => "BXOR",
            Opcode::Bnot => "BNOT",
            Opcode::Shl => "SHL",
            Opcode::Shr => "SHR",
            Opcode::Ushr => "USHR",
            Opcode::AddG => "ADD_G",
            Opcode::SubG => "SUB_G",
            Opcode::MulG => "MUL_G",
            Opcode::DivG => "DIV_G",
            // Comparison (0x40-0x4F)
            Opcode::EqI => "EQ_I",
            Opcode::NeI => "NE_I",
            Opcode::LtI => "LT_I",
            Opcode::LeI => "LE_I",
            Opcode::GtI => "GT_I",
            Opcode::GeI => "GE_I",
            Opcode::EqF => "EQ_F",
            Opcode::NeF => "NE_F",
            Opcode::LtF => "LT_F",
            Opcode::LeF => "LE_F",
            Opcode::GtF => "GT_F",
            Opcode::GeF => "GE_F",
            Opcode::EqG => "EQ_G",
            Opcode::CmpG => "CMP_G",
            Opcode::EqRef => "EQ_REF",
            Opcode::CmpExtended => "CMP_EXTENDED",
            // Control Flow (0x50-0x5F)
            Opcode::Jmp => "JMP",
            Opcode::JmpIf => "JMP_IF",
            Opcode::JmpNot => "JMP_NOT",
            Opcode::JmpEq => "JMP_EQ",
            Opcode::JmpNe => "JMP_NE",
            Opcode::JmpLt => "JMP_LT",
            Opcode::JmpLe => "JMP_LE",
            Opcode::JmpGt => "JMP_GT",
            Opcode::JmpGe => "JMP_GE",
            Opcode::Ret => "RET",
            Opcode::RetV => "RET_V",
            Opcode::Call => "CALL",
            Opcode::TailCall => "TAIL_CALL",
            Opcode::CallM => "CALL_M",
            Opcode::CallClosure => "CALL_CLOSURE",
            Opcode::CallR => "CALL_R",
            // Memory + Collections (0x60-0x6F)
            Opcode::New => "NEW",
            Opcode::NewG => "NEW_G",
            Opcode::GetF => "GET_F",
            Opcode::SetF => "SET_F",
            Opcode::GetE => "GET_E",
            Opcode::SetE => "SET_E",
            Opcode::Len => "LEN",
            Opcode::NewArray => "NEW_ARRAY",
            Opcode::NewList => "NEW_LIST",
            Opcode::ListPush => "LIST_PUSH",
            Opcode::ListPop => "LIST_POP",
            Opcode::NewMap => "NEW_MAP",
            Opcode::MapGet => "MAP_GET",
            Opcode::MapSet => "MAP_SET",
            Opcode::MapContains => "MAP_CONTAINS",
            Opcode::Clone => "CLONE",
            // CBGR (0x70-0x7F)
            Opcode::Ref => "REF",
            Opcode::RefMut => "REF_MUT",
            Opcode::Deref => "DEREF",
            Opcode::DerefMut => "DEREF_MUT",
            Opcode::ChkRef => "CHK_REF",
            Opcode::RefChecked => "REF_CHECKED",
            Opcode::RefUnsafe => "REF_UNSAFE",
            Opcode::DropRef => "DROP_REF",
            Opcode::CbgrExtended => "CBGR_EXTENDED",
            Opcode::TextExtended => "TEXT_EXTENDED",
            // Generic + Variant (0x80-0x8F)
            Opcode::CallG => "CALL_G",
            Opcode::CallV => "CALL_V",
            Opcode::CallC => "CALL_C",
            Opcode::SizeOfG => "SIZE_OF_G",
            Opcode::AlignOfG => "ALIGN_OF_G",
            Opcode::Instantiate => "INSTANTIATE",
            Opcode::MakeVariant => "MAKE_VARIANT",
            Opcode::SetVariantData => "SET_VARIANT_DATA",
            Opcode::GetVariantData => "GET_VARIANT_DATA",
            Opcode::GetVariantDataRef => "GET_VARIANT_DATA_REF",
            Opcode::GetTag => "GET_TAG",
            Opcode::TypeOf => "TYPE_OF",
            Opcode::MakePi => "MAKE_PI",
            Opcode::MakeSigma => "MAKE_SIGMA",
            Opcode::MakeWitness => "MAKE_WITNESS",
            Opcode::NewClosure => "NEW_CLOSURE",
            // Pattern + Logic (0x90-0x9F)
            Opcode::IsVar => "IS_VAR",
            Opcode::AsVar => "AS_VAR",
            Opcode::Unpack => "UNPACK",
            Opcode::Pack => "PACK",
            Opcode::Switch => "SWITCH",
            Opcode::MatchGuard => "MATCH_GUARD",
            Opcode::And => "AND",
            Opcode::Or => "OR",
            Opcode::Xor => "XOR",
            Opcode::Not => "NOT",
            // Async + Nursery (0xA0-0xAF)
            Opcode::Spawn => "SPAWN",
            Opcode::Await => "AWAIT",
            Opcode::Yield => "YIELD",
            Opcode::Select => "SELECT",
            Opcode::Join => "JOIN",
            Opcode::FutureReady => "FUTURE_READY",
            Opcode::FutureGet => "FUTURE_GET",
            Opcode::AsyncNext => "ASYNC_NEXT",
            Opcode::NurseryInit => "NURSERY_INIT",
            Opcode::NurserySpawn => "NURSERY_SPAWN",
            Opcode::NurseryAwait => "NURSERY_AWAIT",
            Opcode::NurseryCancel => "NURSERY_CANCEL",
            Opcode::NurseryConfig => "NURSERY_CONFIG",
            Opcode::NurseryError => "NURSERY_ERROR",
            // Context + Meta (0xB0-0xBF)
            Opcode::CtxGet => "CTX_GET",
            Opcode::CtxProvide => "CTX_PROVIDE",
            Opcode::CtxEnd => "CTX_END",
            Opcode::PushContext => "PUSH_CONTEXT",
            Opcode::PopContext => "POP_CONTEXT",
            Opcode::Attenuate => "ATTENUATE",
            Opcode::HasCapability => "HAS_CAP",
            Opcode::RequireCapability => "REQ_CAP",
            Opcode::MetaEval => "META_EVAL",
            Opcode::MetaQuote => "META_QUOTE",
            Opcode::MetaSplice => "META_SPLICE",
            Opcode::MetaReflect => "META_REFLECT",
            Opcode::FfiExtended => "FFI_EXTENDED",
            Opcode::ArithExtended => "ARITH_EXTENDED",
            // Iterator + Generator + String + Set (0xC0-0xCF)
            Opcode::IterNew => "ITER_NEW",
            Opcode::IterNext => "ITER_NEXT",
            Opcode::GenCreate => "GEN_CREATE",
            Opcode::GenNext => "GEN_NEXT",
            Opcode::GenHasNext => "GEN_HAS_NEXT",
            Opcode::ToString => "TO_STRING",
            Opcode::Concat => "CONCAT",
            Opcode::NewSet => "NEW_SET",
            Opcode::SetInsert => "SET_INSERT",
            Opcode::SetContains => "SET_CONTAINS",
            Opcode::SetRemove => "SET_REMOVE",
            Opcode::CharToStr => "CHAR_TO_STR",
            Opcode::NewRange => "NEW_RANGE",
            Opcode::NewDeque => "NEW_DEQUE",
            // Exception + Debug (0xD0-0xDF)
            Opcode::Throw => "THROW",
            Opcode::TryBegin => "TRY_BEGIN",
            Opcode::TryEnd => "TRY_END",
            Opcode::GetException => "GET_EXCEPTION",
            Opcode::Spec => "SPEC",
            Opcode::Guard => "GUARD",
            Opcode::Assert => "ASSERT",
            Opcode::Panic => "PANIC",
            Opcode::Unreachable => "UNREACHABLE",
            Opcode::DebugPrint => "DEBUG_PRINT",
            Opcode::Requires => "REQUIRES",
            Opcode::Ensures => "ENSURES",
            Opcode::Invariant => "INVARIANT",
            Opcode::NewChannel => "NEW_CHANNEL",
            Opcode::CubicalExtended => "CUBICAL_EXTENDED",
            // System (V-LLSI) + Autodiff (0xE0-0xEF)
            Opcode::SyscallLinux => "SYSCALL_LINUX",
            Opcode::Mmap => "MMAP",
            Opcode::Munmap => "MUNMAP",
            Opcode::AtomicLoad => "ATOMIC_LOAD",
            Opcode::AtomicStore => "ATOMIC_STORE",
            Opcode::AtomicCas => "ATOMIC_CAS",
            Opcode::AtomicFence => "ATOMIC_FENCE",
            Opcode::IoSubmit => "IO_SUBMIT",
            Opcode::IoPoll => "IO_POLL",
            Opcode::TlsGet => "TLS_GET",
            Opcode::TlsSet => "TLS_SET",
            Opcode::GradBegin => "GRAD_BEGIN",
            Opcode::GradEnd => "GRAD_END",
            Opcode::GradCheckpoint => "GRAD_CHECKPOINT",
            Opcode::GradAccumulate => "GRAD_ACCUMULATE",
            Opcode::GradStop => "GRAD_STOP",
            // Tensor + GPU (0xF0-0xFF)
            Opcode::TensorNew => "TENSOR_NEW",
            Opcode::TensorBinop => "TENSOR_BINOP",
            Opcode::TensorUnop => "TENSOR_UNOP",
            Opcode::TensorMatmul => "TENSOR_MATMUL",
            Opcode::TensorReduce => "TENSOR_REDUCE",
            Opcode::TensorReshape => "TENSOR_RESHAPE",
            Opcode::TensorTranspose => "TENSOR_TRANSPOSE",
            Opcode::TensorSlice => "TENSOR_SLICE",
            Opcode::GpuExtended => "GPU_EXTENDED",
            Opcode::GpuSync => "GPU_SYNC",
            Opcode::GpuMemcpy => "GPU_MEMCPY",
            Opcode::GpuAlloc => "GPU_ALLOC",
            Opcode::TensorExtended => "TENSOR_EXTENDED",
            Opcode::MlExtended => "ML_EXTENDED",
            Opcode::Extended => "EXTENDED",
            _ => "RESERVED",
        }
    }

    /// Returns true if this opcode is a jump/branch instruction.
    pub fn is_branch(self) -> bool {
        matches!(
            self,
            Opcode::Jmp
                | Opcode::JmpIf
                | Opcode::JmpNot
                | Opcode::JmpEq
                | Opcode::JmpNe
                | Opcode::JmpLt
                | Opcode::JmpLe
                | Opcode::JmpGt
                | Opcode::JmpGe
                | Opcode::Switch
        )
    }

    /// Returns true if this opcode is a return instruction.
    pub fn is_return(self) -> bool {
        matches!(self, Opcode::Ret | Opcode::RetV)
    }

    /// Returns true if this opcode is a call instruction.
    pub fn is_call(self) -> bool {
        matches!(
            self,
            Opcode::Call
                | Opcode::TailCall
                | Opcode::CallM
                | Opcode::CallClosure
                | Opcode::CallR
                | Opcode::CallG
                | Opcode::CallV
                | Opcode::CallC
        )
    }

    /// Returns true if this opcode is a tensor operation.
    pub fn is_tensor(self) -> bool {
        // Tensor ops are now in 0xF0-0xFF range
        (0xF0..=0xFF).contains(&(self as u8))
    }

    /// Returns true if this opcode is a GPU operation.
    pub fn is_gpu(self) -> bool {
        matches!(
            self,
            Opcode::GpuExtended | Opcode::GpuSync | Opcode::GpuMemcpy | Opcode::GpuAlloc
        )
    }

    /// Returns true if this opcode is a system/V-LLSI operation.
    pub fn is_system(self) -> bool {
        matches!(
            self,
            Opcode::SyscallLinux
                | Opcode::Mmap
                | Opcode::Munmap
                | Opcode::AtomicLoad
                | Opcode::AtomicStore
                | Opcode::AtomicCas
                | Opcode::AtomicFence
                | Opcode::IoSubmit
                | Opcode::IoPoll
                | Opcode::TlsGet
                | Opcode::TlsSet
        )
    }
}

/// Full instruction with opcode and operands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    // ========================================================================
    // Data Movement
    // ========================================================================
    /// Move: `dst = src`
    Mov {
        /// Destination register.
        dst: Reg,
        /// Source register.
        src: Reg,
    },
    /// Load constant: `dst = const_pool[id]`
    LoadK {
        /// Destination register.
        dst: Reg,
        /// Index into the constant pool.
        const_id: u32,
    },
    /// Load immediate integer.
    LoadI {
        /// Destination register.
        dst: Reg,
        /// Integer value to load.
        value: i64,
    },
    /// Load immediate float.
    LoadF {
        /// Destination register.
        dst: Reg,
        /// Float value to load.
        value: f64,
    },
    /// Load boolean true.
    LoadTrue {
        /// Destination register.
        dst: Reg,
    },
    /// Load boolean false.
    LoadFalse {
        /// Destination register.
        dst: Reg,
    },
    /// Load unit value.
    LoadUnit {
        /// Destination register.
        dst: Reg,
    },
    /// Load type reference.
    LoadT {
        /// Destination register.
        dst: Reg,
        /// Type reference to load.
        type_ref: TypeRef,
    },
    /// Load small immediate (-64..63).
    LoadSmallI {
        /// Destination register.
        dst: Reg,
        /// Small integer value (-64..63).
        value: i8,
    },

    // ========================================================================
    // Type Conversions
    // ========================================================================
    /// Convert Int to Float: `dst = src as Float`
    CvtIF {
        /// Destination register.
        dst: Reg,
        /// Source register (Int value).
        src: Reg,
    },
    /// Convert Float to Int: `dst = src as Int` with rounding mode
    CvtFI {
        /// Conversion mode: 0=trunc, 1=floor, 2=ceil, 3=round
        mode: FloatToIntMode,
        /// Destination register.
        dst: Reg,
        /// Source register (Float value).
        src: Reg,
    },
    /// Convert Int to Char: `dst = src as Char`
    /// Runtime validation: 0 <= src <= 0x10FFFF, not surrogate
    CvtIC {
        /// Destination register.
        dst: Reg,
        /// Source register (Int value).
        src: Reg,
    },
    /// Convert Char to Int: `dst = src as Int`
    /// Returns Unicode codepoint (0 to 0x10FFFF)
    CvtCI {
        /// Destination register.
        dst: Reg,
        /// Source register (Char value).
        src: Reg,
    },
    /// Convert Bool to Int: `dst = src as Int`
    /// false → 0, true → 1
    CvtBI {
        /// Destination register.
        dst: Reg,
        /// Source register (Bool value).
        src: Reg,
    },

    /// Dynamic Convert to Int: runtime type dispatch
    /// Checks source type and converts appropriately:
    /// - Float → truncate to Int
    /// - Bool → 0 or 1
    /// - Char (stored as Int) → identity
    /// - Int → identity
    CvtToI {
        /// Destination register.
        dst: Reg,
        /// Source register (any numeric type).
        src: Reg,
    },

    /// Dynamic Convert to Float: runtime type dispatch
    /// Checks source type and converts appropriately:
    /// - Int → Float
    /// - Float → identity
    CvtToF {
        /// Destination register.
        dst: Reg,
        /// Source register (Int or Float).
        src: Reg,
    },

    // ========================================================================
    // Arithmetic
    // ========================================================================
    /// Integer arithmetic: `dst = a op b`
    BinaryI {
        /// Binary integer operation type.
        op: BinaryIntOp,
        /// Destination register.
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },
    /// Float arithmetic: `dst = a op b`
    BinaryF {
        /// Binary float operation type.
        op: BinaryFloatOp,
        /// Destination register.
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },
    /// Generic arithmetic via protocol.
    BinaryG {
        /// Binary generic operation type.
        op: BinaryGenericOp,
        /// Destination register.
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
        /// Protocol table index for the operation.
        protocol_id: u32,
    },
    /// Unary integer: `dst = op(src)`
    UnaryI {
        /// Unary integer operation type.
        op: UnaryIntOp,
        /// Destination register.
        dst: Reg,
        /// Source operand register.
        src: Reg,
    },
    /// Unary float: `dst = op(src)`
    UnaryF {
        /// Unary float operation type.
        op: UnaryFloatOp,
        /// Destination register.
        dst: Reg,
        /// Source operand register.
        src: Reg,
    },
    /// Boolean not: `dst = !src`
    Not {
        /// Destination register.
        dst: Reg,
        /// Source operand register.
        src: Reg,
    },
    /// Bitwise operation.
    Bitwise {
        /// Bitwise operation type.
        op: BitwiseOp,
        /// Destination register.
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register (ignored for NOT).
        b: Reg,
    },

    // ========================================================================
    // Comparison
    // ========================================================================
    /// Integer comparison: `dst = a cmp b`
    CmpI {
        /// Comparison operation type.
        op: CompareOp,
        /// Destination register (boolean result).
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },
    /// Float comparison: `dst = a cmp b`
    CmpF {
        /// Comparison operation type.
        op: CompareOp,
        /// Destination register (boolean result).
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },
    /// Generic comparison via Eq/Ord protocol.
    CmpG {
        /// True for equality (Eq protocol), false for ordering (Ord protocol).
        eq: bool,
        /// Destination register (boolean or Ordering result).
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
        /// Protocol table index for the comparison.
        protocol_id: u32,
    },
    /// Unsigned integer comparison: `dst = a cmp_unsigned b`
    ///
    /// Interprets operands as unsigned 64-bit integers for ordering comparisons.
    /// Encoded via CmpExtended (0x4F) prefix with CmpSubOpcode.
    CmpU {
        /// Comparison sub-opcode (LtU, LeU, GtU, GeU).
        sub_op: CmpSubOpcode,
        /// Destination register (boolean result).
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },

    // ========================================================================
    // Control Flow
    // ========================================================================
    /// No operation.
    Nop,
    /// Unconditional jump.
    Jmp {
        /// Signed offset in instructions from current position.
        offset: i32,
    },
    /// Conditional jump if true.
    JmpIf {
        /// Condition register (boolean).
        cond: Reg,
        /// Signed offset in instructions from current position.
        offset: i32,
    },
    /// Conditional jump if false.
    JmpNot {
        /// Condition register (boolean).
        cond: Reg,
        /// Signed offset in instructions from current position.
        offset: i32,
    },
    /// Fused compare-and-jump.
    JmpCmp {
        /// Comparison operation type.
        op: CompareOp,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
        /// Signed offset in instructions from current position.
        offset: i32,
    },
    /// Return with value.
    Ret {
        /// Register containing return value.
        value: Reg,
    },
    /// Return void.
    RetV,
    /// Call function.
    Call {
        /// Destination register for return value.
        dst: Reg,
        /// Function table index.
        func_id: u32,
        /// Argument registers.
        args: RegRange,
    },
    /// Tail call (reuses stack frame).
    TailCall {
        /// Function table index.
        func_id: u32,
        /// Argument registers.
        args: RegRange,
    },
    /// Method call.
    CallM {
        /// Destination register for return value.
        dst: Reg,
        /// Object register for method receiver.
        receiver: Reg,
        /// Method table index.
        method_id: u32,
        /// Argument registers (excluding receiver).
        args: RegRange,
    },
    /// Call closure.
    CallClosure {
        /// Destination register for return value.
        dst: Reg,
        /// Closure register.
        closure: Reg,
        /// Argument registers.
        args: RegRange,
    },
    /// Create closure.
    ///
    /// Creates a new closure object on the heap with the specified function ID
    /// and captured values. The closure layout in memory is:
    /// - function_id: u32 (4 bytes)
    /// - capture_count: u16 (2 bytes)
    /// - padding: u16 (2 bytes)
    /// - captures: [Value; capture_count] (8 bytes each)
    ///
    /// # Operands
    /// - `dst`: Destination register for the closure pointer
    /// - `func_id`: Function table index for the closure body
    /// - `captures`: Registers containing captured values (in order)
    NewClosure {
        /// Destination register for closure pointer.
        dst: Reg,
        /// Function table index for the closure body.
        func_id: u32,
        /// Registers containing captured values.
        captures: Vec<Reg>,
    },

    // ========================================================================
    // Memory
    // ========================================================================
    /// Allocate object of type.
    New {
        /// Destination register for new object.
        dst: Reg,
        /// Type table index.
        type_id: u32,
        /// Number of Value slots to allocate for this object's data area.
        /// For record types, this is max(field_indices) + 1.
        field_count: u32,
    },
    /// Allocate generic type.
    NewG {
        /// Destination register for new object.
        dst: Reg,
        /// Type table index.
        type_id: u32,
        /// Registers containing type arguments.
        type_args: Vec<Reg>,
    },
    /// Get field value.
    GetF {
        /// Destination register for field value.
        dst: Reg,
        /// Object register.
        obj: Reg,
        /// Field index within the type.
        field_idx: u32,
    },
    /// Set field value.
    SetF {
        /// Object register.
        obj: Reg,
        /// Field index within the type.
        field_idx: u32,
        /// Value register to assign.
        value: Reg,
    },
    /// Get array element.
    GetE {
        /// Destination register for element value.
        dst: Reg,
        /// Array register.
        arr: Reg,
        /// Index register.
        idx: Reg,
    },
    /// Set array element.
    SetE {
        /// Array register.
        arr: Reg,
        /// Index register.
        idx: Reg,
        /// Value register to assign.
        value: Reg,
    },
    /// Get length.
    Len {
        /// Destination register for length.
        dst: Reg,
        /// Array or collection register.
        arr: Reg,
        /// Type hint for the collection (0=unknown, 1=List, 2=Map, 3=Set, 4=Deque, 5=Text, 6=Channel, 7=Slice).
        /// Allows LLVM lowering to read from the correct offset without relying on
        /// register tracking (which is unreliable after function calls clear type marks).
        type_hint: u8,
    },
    /// Create new list.
    NewList {
        /// Destination register for new list.
        dst: Reg,
    },
    /// Push value to list.
    ListPush {
        /// List register.
        list: Reg,
        /// Value register to push.
        val: Reg,
    },
    /// Pop value from list.
    ListPop {
        /// Destination register for popped value.
        dst: Reg,
        /// List register.
        list: Reg,
    },
    /// Create new map.
    NewMap {
        /// Destination register for new map.
        dst: Reg,
    },
    /// Get value from map.
    MapGet {
        /// Destination register for value.
        dst: Reg,
        /// Map register.
        map: Reg,
        /// Key register.
        key: Reg,
    },
    /// Set value in map.
    MapSet {
        /// Map register.
        map: Reg,
        /// Key register.
        key: Reg,
        /// Value register.
        val: Reg,
    },
    /// Check if map contains key.
    MapContains {
        /// Destination register (boolean result).
        dst: Reg,
        /// Map register.
        map: Reg,
        /// Key register.
        key: Reg,
    },
    /// Create iterator from iterable.
    IterNew {
        /// Destination register for iterator.
        dst: Reg,
        /// Iterable register.
        iterable: Reg,
    },
    /// Get next element from iterator.
    IterNext {
        /// Destination register for element.
        dst: Reg,
        /// Destination register for has_next flag.
        has_next: Reg,
        /// Iterator register.
        iter: Reg,
    },
    /// Clone value.
    Clone {
        /// Destination register for cloned value.
        dst: Reg,
        /// Source register to clone.
        src: Reg,
    },

    // ========================================================================
    // CBGR
    // ========================================================================
    /// Create immutable reference.
    Ref {
        /// Destination register for reference.
        dst: Reg,
        /// Source value register.
        src: Reg,
    },
    /// Create mutable reference.
    RefMut {
        /// Destination register for mutable reference.
        dst: Reg,
        /// Source value register.
        src: Reg,
    },
    /// Dereference (read through reference).
    Deref {
        /// Destination register for dereferenced value.
        dst: Reg,
        /// Reference register to dereference.
        ref_reg: Reg,
    },
    /// Dereference mutable (write through reference).
    ///
    /// Stores a value through a mutable reference: `*ref_reg = value`
    DerefMut {
        /// Mutable reference register.
        ref_reg: Reg,
        /// Value to store.
        value: Reg,
    },
    /// CBGR validation check.
    ChkRef {
        /// Reference register to validate.
        ref_reg: Reg,
    },
    /// Create checked (Tier 1) reference.
    RefChecked {
        /// Destination register for checked reference.
        dst: Reg,
        /// Source value register.
        src: Reg,
    },
    /// Create unsafe (Tier 2) reference.
    RefUnsafe {
        /// Destination register for unsafe reference.
        dst: Reg,
        /// Source value register.
        src: Reg,
    },
    /// Drop a value, bumping CBGR generation to invalidate references.
    DropRef {
        /// Register holding the value to drop.
        src: Reg,
    },

    // ========================================================================
    // Generic
    // ========================================================================
    /// Generic function call.
    CallG {
        /// Destination register for return value.
        dst: Reg,
        /// Function table index.
        func_id: u32,
        /// Registers containing type arguments.
        type_args: Vec<Reg>,
        /// Argument registers.
        args: RegRange,
    },
    /// Virtual dispatch.
    CallV {
        /// Destination register for return value.
        dst: Reg,
        /// Object register for method receiver.
        receiver: Reg,
        /// Vtable slot index.
        vtable_slot: u32,
        /// Argument registers (excluding receiver).
        args: RegRange,
    },
    /// Inline cached call.
    CallC {
        /// Destination register for return value.
        dst: Reg,
        /// Inline cache identifier.
        cache_id: u32,
        /// Argument registers.
        args: RegRange,
    },

    // ========================================================================
    // Pattern Matching
    // ========================================================================
    /// Check if value is specific variant.
    IsVar {
        /// Destination register (boolean result).
        dst: Reg,
        /// Value register to check.
        value: Reg,
        /// Variant tag to match against.
        tag: u32,
    },
    /// Extract variant payload.
    AsVar {
        /// Destination register for payload.
        dst: Reg,
        /// Value register containing variant.
        value: Reg,
        /// Variant tag to extract.
        tag: u32,
    },
    /// Unpack tuple into consecutive registers.
    Unpack {
        /// First destination register.
        dst_start: Reg,
        /// Tuple register to unpack.
        tuple: Reg,
        /// Number of elements to unpack.
        count: u8,
    },
    /// Pack consecutive registers into tuple.
    Pack {
        /// Destination register for tuple.
        dst: Reg,
        /// First source register.
        src_start: Reg,
        /// Number of elements to pack.
        count: u8,
    },
    /// Switch/jump table.
    Switch {
        /// Value register to switch on.
        value: Reg,
        /// Signed offset for default case.
        default_offset: i32,
        /// Vector of (case_value, offset) pairs.
        cases: Vec<(u32, i32)>,
    },

    // ========================================================================
    // Generator Operations (Iterator Protocol)
    // ========================================================================
    //
    // Generator State Machine: `fn*` functions compile to generator bytecode.
    // GenCreate (0x9E) allocates a Generator struct with saved_pc/registers/contexts.
    // GenNext (0x9F) resumes execution from the saved state; Yield saves state and returns.
    // GenHasNext (0xC9) checks if the generator can produce more values (status != Completed).
    // Generators implement the Iterator protocol for lazy value production.

    /// Create a generator from a generator function.
    ///
    /// Creates a new generator instance that can be iterated via GenNext.
    /// The generator starts in Created state and must be resumed to begin execution.
    /// Arguments are stored in the generator's initial register state for use when
    /// first resumed via GenNext.
    GenCreate {
        /// Destination register for generator value.
        dst: Reg,
        /// Generator function ID.
        func_id: u32,
        /// Argument registers to initialize the generator with.
        args: RegRange,
    },

    /// Get the next value from a generator (Iterator::next).
    ///
    /// Returns the next yielded value wrapped in Some, or None if exhausted.
    /// The result is stored as a variant: Some(value) if yielded, None if completed.
    GenNext {
        /// Destination register for Option<Value> result.
        dst: Reg,
        /// Generator value register.
        generator: Reg,
    },

    /// Check if a generator has more values (Iterator::has_next).
    ///
    /// Returns true if the generator can produce more values, false otherwise.
    GenHasNext {
        /// Destination register for boolean result.
        dst: Reg,
        /// Generator value register.
        generator: Reg,
    },

    // ========================================================================
    // Async
    // ========================================================================
    /// Spawn async task.
    Spawn {
        /// Destination register for task handle.
        dst: Reg,
        /// Async function table index.
        func_id: u32,
        /// Argument registers.
        args: RegRange,
    },
    /// Await task.
    Await {
        /// Destination register for task result.
        dst: Reg,
        /// Task handle register.
        task: Reg,
    },
    /// Yield value.
    Yield {
        /// Value register to yield.
        value: Reg,
    },
    /// Select on multiple futures.
    Select {
        /// Destination register for completed future index.
        dst: Reg,
        /// Registers containing futures.
        futures: Vec<Reg>,
        /// Jump offsets for each future's handler.
        handlers: Vec<i32>,
    },

    // ========================================================================
    // Structured Concurrency (Nursery)
    // ========================================================================
    /// Initialize a new nursery scope for structured concurrency.
    /// The nursery tracks all spawned tasks and ensures they complete
    /// before the scope exits.
    NurseryInit {
        /// Destination register for nursery handle.
        dst: Reg,
    },
    /// Set timeout for nursery scope.
    NurserySetTimeout {
        /// Nursery handle register.
        nursery: Reg,
        /// Timeout value register (Duration).
        timeout: Reg,
    },
    /// Set maximum concurrent tasks for nursery.
    NurserySetMaxTasks {
        /// Nursery handle register.
        nursery: Reg,
        /// Maximum tasks register (Int).
        max_tasks: Reg,
    },
    /// Set error handling behavior for nursery.
    /// behavior values: 0 = CancelAll, 1 = WaitAll, 2 = FailFast
    NurserySetErrorBehavior {
        /// Nursery handle register.
        nursery: Reg,
        /// Behavior code register.
        behavior: Reg,
    },
    /// Enter nursery scope (push onto runtime context).
    NurseryEnter {
        /// Nursery handle register.
        nursery: Reg,
    },
    /// Exit nursery scope (pop from runtime context).
    NurseryExit {
        /// Nursery handle register.
        nursery: Reg,
    },
    /// Spawn a task into the current nursery.
    NurserySpawn {
        /// Destination register for task handle.
        dst: Reg,
        /// Nursery handle register.
        nursery: Reg,
        /// Task function closure register.
        task: Reg,
    },
    /// Wait for all tasks in nursery to complete.
    /// Returns success=true if all tasks completed without error.
    NurseryAwaitAll {
        /// Nursery handle register.
        nursery: Reg,
        /// Destination register for success boolean.
        success: Reg,
    },
    /// Get the collected error from nursery (if any task failed).
    NurseryGetError {
        /// Nursery handle register.
        nursery: Reg,
        /// Destination register for error value.
        dst: Reg,
    },
    /// Cancel all running tasks in nursery.
    NurseryCancel {
        /// Nursery handle register.
        nursery: Reg,
    },

    // ========================================================================
    // Autodiff
    // ========================================================================
    /// Begin gradient scope.
    GradBegin {
        /// Gradient scope identifier.
        scope_id: u32,
        /// Differentiation mode (forward, reverse, auto).
        mode: GradMode,
        /// Registers of variables to differentiate with respect to.
        wrt: Vec<Reg>,
    },
    /// End gradient scope and compute.
    GradEnd {
        /// Gradient scope identifier.
        scope_id: u32,
        /// Output tensor register.
        output: Reg,
        /// Output gradient register (for reverse mode).
        grad_out: Reg,
        /// Destination registers for computed gradients.
        grad_regs: Vec<Reg>,
    },
    /// Gradient checkpoint.
    GradCheckpoint {
        /// Checkpoint identifier.
        id: u32,
        /// Tensor registers to checkpoint.
        tensors: Vec<Reg>,
    },
    /// Accumulate gradients.
    GradAccumulate {
        /// Destination register for accumulated gradient.
        dst: Reg,
        /// Source gradient register to add.
        src: Reg,
    },
    /// Stop gradient flow (detach).
    GradStop {
        /// Destination register for detached tensor.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
    },

    // ========================================================================
    // Context
    // ========================================================================
    /// Get context value.
    CtxGet {
        /// Destination register for context value.
        dst: Reg,
        /// Context type table index.
        ctx_type: u32,
    },
    /// Provide context for body.
    CtxProvide {
        /// Context type table index.
        ctx_type: u32,
        /// Context value register.
        value: Reg,
        /// Signed offset to body end instruction.
        body_offset: i32,
    },
    /// End context scope (for scoped provide).
    CtxEnd,

    /// Assert that a context is NOT present in the context stack (negative constraint).
    ///
    /// Emitted at function entry for each `!Context` in the `using` clause.
    /// At runtime, checks the context stack and panics if the excluded context
    /// is found, enforcing the negative context constraint declared by the function.
    CtxCheckNegative {
        /// Context type table index (the context that must NOT be present).
        ctx_type: u32,
        /// Interned function name (for error messages).
        func_name: u32,
    },

    // ========================================================================
    // Debug/Verify
    // ========================================================================
    /// Type assertion hint.
    Spec {
        /// Register to specialize.
        reg: Reg,
        /// Expected type table index.
        expected_type: u32,
    },
    /// Type guard with deoptimization.
    Guard {
        /// Register to guard.
        reg: Reg,
        /// Expected type table index.
        expected_type: u32,
        /// Signed offset to deoptimization handler.
        deopt_offset: i32,
    },
    /// Assert condition.
    Assert {
        /// Condition register (boolean).
        cond: Reg,
        /// Message string pool index.
        message_id: u32,
    },
    /// Panic with message.
    Panic {
        /// Message string pool index.
        message_id: u32,
    },
    /// Unreachable marker.
    Unreachable,
    /// Debug print value.
    DebugPrint {
        /// Value register to print.
        value: Reg,
    },

    // ========================================================================
    // Set Operations
    // ========================================================================
    /// Create new set.
    NewSet {
        /// Destination register for new set.
        dst: Reg,
    },

    // ========================================================================
    // Deque Operations
    // ========================================================================
    /// Create new empty deque with default capacity.
    /// Layout: [data, head, len, cap] — matches
    /// `type Deque<T> is { data, head, len, cap }` in
    /// `core/collections/deque.vr`. Interpreter handler
    /// (`handle_new_deque` @ opcode 0xCD) allocates with
    /// `TypeId::DEQUE` so every subsequent `push_back`/`pop_back`/…
    /// dispatches through the builtin Deque handlers instead of
    /// the stdlib's raw-pointer `self.data.offset(…)` path (which
    /// can't work against the builtin layout).
    NewDeque {
        /// Destination register for new deque.
        dst: Reg,
    },
    /// Insert element into set.
    SetInsert {
        /// Set register.
        set: Reg,
        /// Element register to insert.
        elem: Reg,
    },
    /// Check if set contains element.
    SetContains {
        /// Destination register (boolean result).
        dst: Reg,
        /// Set register.
        set: Reg,
        /// Element register to check.
        elem: Reg,
    },
    /// Remove element from set.
    SetRemove {
        /// Set register.
        set: Reg,
        /// Element register to remove.
        elem: Reg,
    },

    // ========================================================================
    // String Operations
    // ========================================================================
    /// Convert value to string.
    ToString {
        /// Destination register for string.
        dst: Reg,
        /// Source value register.
        src: Reg,
    },
    /// Concatenate strings.
    Concat {
        /// Destination register for concatenated string.
        dst: Reg,
        /// First string register.
        a: Reg,
        /// Second string register.
        b: Reg,
    },
    /// Convert Char to string (1-character string).
    CharToStr {
        /// Destination register for string.
        dst: Reg,
        /// Source Char register (stored as Int).
        src: Reg,
    },

    /// Create a new range for iteration.
    /// Range layout: [start: Int, end: Int, step: Int (always 1), inclusive: Bool]
    NewRange {
        /// Destination register for new range.
        dst: Reg,
        /// Start value register.
        start: Reg,
        /// End value register.
        end: Reg,
        /// Whether range is inclusive (includes end value).
        inclusive: bool,
    },

    // ========================================================================
    // Stack Operations
    // ========================================================================
    /// Push value to argument stack.
    Push {
        /// Source register to push.
        src: Reg,
    },
    /// Pop value from argument stack.
    Pop {
        /// Destination register for popped value.
        dst: Reg,
    },

    // ========================================================================
    // Indirect Calls
    // ========================================================================
    /// Call via register (indirect call).
    CallR {
        /// Destination register for return value.
        dst: Reg,
        /// Function/closure register.
        func: Reg,
        /// Number of arguments.
        argc: u8,
    },

    // ========================================================================
    // Nil/None Value
    // ========================================================================
    /// Load nil/null value.
    LoadNil {
        /// Destination register.
        dst: Reg,
    },

    // ========================================================================
    // Exception Handling
    // ========================================================================
    /// Throw exception.
    Throw {
        /// Error value register.
        error: Reg,
    },
    /// Begin try block (sets up exception handler).
    TryBegin {
        /// Signed offset to exception handler from this instruction.
        handler_offset: i32,
    },
    /// End try block (clears exception handler).
    TryEnd,
    /// Get caught exception value.
    GetException {
        /// Destination register for exception value.
        dst: Reg,
    },

    // ========================================================================
    // Future/Async Operations (extended)
    // ========================================================================
    /// Check if future is ready (poll).
    FutureReady {
        /// Destination register (boolean result).
        dst: Reg,
        /// Future register to check.
        future: Reg,
    },
    /// Get future result (blocking if not ready).
    FutureGet {
        /// Destination register for result.
        dst: Reg,
        /// Future register.
        future: Reg,
    },
    /// Get next element from async iterator.
    AsyncNext {
        /// Destination register for Option<T> result.
        dst: Reg,
        /// Async iterator register.
        iter: Reg,
    },

    // ========================================================================
    // Context System (extended)
    // ========================================================================
    /// Push context handler onto context stack.
    PushContext {
        /// Context type name index.
        name: u32,
        /// Handler/value register.
        handler: Reg,
    },
    /// Pop context handler from context stack.
    PopContext {
        /// Context type name index.
        name: u32,
    },
    /// Attenuate context capabilities.
    Attenuate {
        /// Destination register for attenuated context.
        dst: Reg,
        /// Source context register.
        context: Reg,
        /// Capability mask string index.
        capabilities: u32,
    },

    // ========================================================================
    // Meta-Programming
    // ========================================================================
    /// Create TokenStream from serialized token data.
    ///
    /// Quote expressions compile to this instruction. The serialized bytes
    /// are stored in the constant pool and decoded at runtime to create
    /// a TokenStream heap object.
    ///
    /// Format: `dst = quote(bytes_const_id)`
    ///
    /// Staged meta-compilation: captures a code fragment as a TokenStream at compile time.
    /// The serialized token data is stored in the constant pool and deserialized at expansion.
    /// Used by `@` macro invocations and `meta fn` procedural macros.
    MetaQuote {
        /// Destination register for TokenStream object.
        dst: Reg,
        /// Constant pool index containing serialized token data.
        bytes_const_id: u32,
    },
    /// Splice TokenStream into code.
    ///
    /// Used during meta expansion to insert generated code.
    MetaSplice {
        /// TokenStream to splice.
        src: Reg,
    },
    /// Evaluate meta expression.
    ///
    /// Forces compile-time evaluation of the expression.
    MetaEval {
        /// Destination register for evaluated result.
        dst: Reg,
        /// Expression to evaluate.
        expr: Reg,
    },
    /// Reflect on type information.
    ///
    /// Returns type metadata as a runtime value.
    MetaReflect {
        /// Destination register for type info.
        dst: Reg,
        /// Type table index.
        type_id: u32,
    },

    // ========================================================================
    // Collection Operations (extended)
    // ========================================================================
    /// Create iterator from iterable (simplified IterNew).
    Iter {
        /// Destination register for iterator.
        dst: Reg,
        /// Iterable register.
        iterable: Reg,
    },
    /// Get variant tag.
    GetTag {
        /// Destination register for tag value (u32).
        dst: Reg,
        /// Variant register.
        variant: Reg,
    },
    /// Create variant with tag and allocate space for fields.
    MakeVariant {
        /// Destination register for variant.
        dst: Reg,
        /// Variant tag.
        tag: u32,
        /// Number of payload fields to allocate space for.
        field_count: u32,
    },
    /// Set variant data field.
    SetVariantData {
        /// Variant register.
        variant: Reg,
        /// Field index within variant.
        field: u32,
        /// Value register to set.
        value: Reg,
    },
    /// Get variant data field.
    GetVariantData {
        /// Destination register for field value.
        dst: Reg,
        /// Variant register.
        variant: Reg,
        /// Field index within variant.
        field: u32,
    },
    /// Get pointer to variant data field (for ref/ref mut pattern bindings).
    /// Unlike GetVariantData which copies the value, this returns a pointer
    /// to the field location enabling mutation through references.
    GetVariantDataRef {
        /// Destination register for field pointer.
        dst: Reg,
        /// Variant register.
        variant: Reg,
        /// Field index within variant.
        field: u32,
    },
    /// **MakePi** — construct a dependent function value `Π(x: T). U(x)`.
    ///
    /// At Tier-0 a dependent function is represented as a 2-slot heap
    /// record tagged `TypeId::PI` with layout `[header | param : Value |
    /// type_id : u64]`. The `param` slot captures the pre-image value
    /// (an ordinary NaN-boxed `Value`) and `type_id` carries the
    /// return-type descriptor's 32-bit id widened to 64 bits so the
    /// interpreter and future reflection tactics can recover the
    /// dependent return type.
    ///
    /// The opcode does *not* perform any dependent-type coercion by
    /// itself — upstream typecheck has already validated the call
    /// site. This is purely the runtime packaging primitive.
    MakePi {
        /// Destination register receiving the packed Pi value.
        dst: Reg,
        /// Register holding the parameter value captured by the Pi.
        param: Reg,
        /// Return-type descriptor id.
        return_type_id: u32,
    },
    /// **MakeSigma** — construct a dependent pair `Σ(x: T). U(x)`.
    ///
    /// At Tier-0 a dependent pair is a 2-slot heap record tagged
    /// `TypeId::SIGMA` with layout `[header | witness : Value |
    /// payload : Value]`. The `witness` is the first component (value
    /// of type `T`) and `payload` the second (of type `U(witness)`).
    /// Projection is performed by `GetVariantData` with field index 0
    /// (witness) or 1 (payload), or by dedicated `PiProj` / `SigmaFst`
    /// / `SigmaSnd` opcodes once they land.
    MakeSigma {
        /// Destination register receiving the packed Sigma pair.
        dst: Reg,
        /// Register holding the first component (the witness).
        witness: Reg,
        /// Register holding the second component (the dependent payload).
        payload: Reg,
    },
    /// **MakeWitness** — attach a proof hash to a refined value.
    ///
    /// At Tier-0 a witness is a 2-slot heap record tagged
    /// `TypeId::WITNESS` with layout `[header | value : Value |
    /// proof_hash : u64]`. The hash is emitted by the static verifier
    /// at compile time and uniquely identifies the proof term that
    /// discharged the refinement obligation. Runtime consumers
    /// (gradual verification boundaries, tier-promotion checks) can
    /// compare hashes to decide whether to re-verify or accept.
    ///
    /// At Tier-1 the opcode is elided when the predicate was
    /// SMT-discharged during compilation — only the underlying value
    /// survives into LLVM IR.
    MakeWitness {
        /// Destination register receiving the packed witness.
        dst: Reg,
        /// Register holding the refined value.
        value: Reg,
        /// 32-bit hash of the static proof term that discharged the
        /// predicate (widened to u64 on the wire for future growth).
        proof_hash: u32,
    },
    /// Make list with initial capacity/length.
    MakeList {
        /// Destination register for new list.
        dst: Reg,
        /// Initial length (0 for empty list).
        len: u16,
    },
    /// Make map with initial capacity.
    MakeMap {
        /// Destination register for new map.
        dst: Reg,
        /// Initial capacity hint.
        capacity: u16,
    },
    /// Make set with initial capacity.
    MakeSet {
        /// Destination register for new set.
        dst: Reg,
        /// Initial capacity hint.
        capacity: u16,
    },
    /// Insert key-value pair into map.
    MapInsert {
        /// Map register.
        map: Reg,
        /// Key register.
        key: Reg,
        /// Value register.
        value: Reg,
    },
    /// Make tensor with shape and data.
    MakeTensor {
        /// Destination register for new tensor.
        dst: Reg,
        /// Number of dimensions.
        shape_len: u16,
        /// Total number of elements.
        total_size: u32,
        /// Data register (flat array of values).
        data: Reg,
    },

    // ========================================================================
    // Tensor Operations (simplified - full tensor ops have more variants)
    // ========================================================================
    /// Create tensor.
    TensorNew {
        /// Destination register for new tensor.
        dst: Reg,
        /// Data type of tensor elements.
        dtype: TensorDType,
        /// Registers containing dimension sizes.
        dims: Vec<Reg>,
    },
    /// Tensor binary operation.
    TensorBinop {
        /// Binary operation type.
        op: TensorBinaryOp,
        /// Destination register for result tensor.
        dst: Reg,
        /// Left operand tensor register.
        a: Reg,
        /// Right operand tensor register.
        b: Reg,
    },
    /// Tensor unary operation.
    TensorUnop {
        /// Unary operation type.
        op: TensorUnaryOp,
        /// Destination register for result tensor.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
    },
    /// Matrix multiplication.
    TensorMatmul {
        /// Destination register for result tensor.
        dst: Reg,
        /// Left matrix tensor register.
        a: Reg,
        /// Right matrix tensor register.
        b: Reg,
    },
    /// Tensor reduction.
    TensorReduce {
        /// Reduction operation type.
        op: TensorReduceOp,
        /// Destination register for result tensor.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axes along which to reduce.
        axes: Vec<u8>,
        /// Whether to keep reduced dimensions as size 1.
        keepdim: bool,
    },
    /// Flash attention.
    TensorFlashAttention {
        /// Destination register for attention output tensor.
        dst: Reg,
        /// Query tensor register.
        q: Reg,
        /// Key tensor register.
        k: Reg,
        /// Value tensor register.
        v: Reg,
        /// Optional attention mask tensor register.
        mask: Option<Reg>,
        /// Scale factor register.
        scale: Reg,
        /// Whether to apply causal masking.
        causal: bool,
    },

    /// Contiguous view of tensor (no copy if possible).
    TensorContiguousView {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
    },

    /// Generate random unsigned 64-bit integer.
    RandomU64 {
        /// Destination register.
        dst: Reg,
    },

    /// Generate random float in custom range [low, high).
    RandomFloat {
        /// Destination register.
        dst: Reg,
        /// Low bound register.
        low: Reg,
        /// High bound register.
        high: Reg,
    },

    /// Get reference to global allocator.
    GlobalAllocator {
        /// Destination register.
        dst: Reg,
    },

    /// Generate new memory allocation ID.
    MemNewId {
        /// Destination register.
        dst: Reg,
    },

    /// Allocate tensor in memory.
    MemAllocTensor {
        /// Destination register.
        dst: Reg,
        /// Shape register.
        shape: Reg,
        /// Data type.
        dtype: u8,
    },

    /// Wire-level bridge to the runtime `PermissionRouter`
    /// (#12 / P3.2). Reads `scope_tag: u32` and `target_id: u64`
    /// from the named registers, routes through
    /// `InterpreterState::check_permission`, and writes the
    /// decision tag (`0` = Allow, `1` = Deny) into `dst`.
    ///
    /// NOT itself permission-gated — gating the gating
    /// intrinsic would create an infinite recursion in the
    /// dispatch path. The Rust-side router holds the warm-path
    /// cache so repeated invocations hit ≤2ns.
    PermissionCheckWire {
        /// Destination register receiving the decision tag.
        dst: Reg,
        /// Register holding the scope wire tag (u32).
        scope_tag: Reg,
        /// Register holding the target id (u64).
        target_id: Reg,
    },

    /// Atomic permission assert (#12 / P3.2).
    ///
    /// Routes the check through the runtime `PermissionRouter`;
    /// on Allow proceeds to the next instruction with no
    /// observable effect, on Deny raises a typed Verum
    /// `PermissionDenied` exception that the surrounding catch
    /// frame can intercept.
    ///
    /// The single-instruction shape lets the codegen emit a
    /// gate prologue without branching machinery — the dispatch
    /// handler holds all the deny-path logic. Designed to be
    /// auto-emitted by the AST→VBC pass before any intrinsic
    /// carrying `IntrinsicHint::RequiresPermission`.
    PermissionAssert {
        /// Compile-time-known scope tag (0..=6 mirroring
        /// `PermissionScope::to_wire_tag`).  Picked at the
        /// emission site from the intrinsic's
        /// `IntrinsicCategory`.
        scope_tag: u8,
        /// Register holding the target id (u64).
        target_id: Reg,
    },

    /// Read a single field of `PermissionRouterStats` (#101).
    /// `selector` selects: 0=total, 1=last_entry_hits,
    /// 2=map_hits, 3=policy_invocations, 4=denials.
    /// Out-of-range selectors return 0.
    PermissionStatsRead {
        /// Destination register receiving the u64 stat.
        dst: Reg,
        /// Register holding the field selector (UInt32).
        selector: Reg,
    },

    /// Reset `PermissionRouter` stats to zero (#101).
    /// Cache itself is preserved.
    PermissionStatsClear {
        /// Destination register receiving Unit.
        dst: Reg,
    },

    // ========================================================================
    // GPU
    // ========================================================================
    /// Launch GPU kernel.
    GpuLaunch {
        /// Kernel function table index.
        kernel_id: u32,
        /// Grid dimensions (x, y, z) registers.
        grid: [Reg; 3],
        /// Block dimensions (x, y, z) registers.
        block: [Reg; 3],
        /// Shared memory size register.
        shared_mem: Reg,
        /// CUDA stream register.
        stream: Reg,
        /// Kernel argument registers.
        args: Vec<Reg>,
    },
    /// Sync GPU stream.
    GpuSync {
        /// CUDA stream register to synchronize.
        stream: Reg,
    },
    /// GPU memory copy (H2D, D2H, D2D).
    GpuMemcpy {
        /// Destination register (tensor).
        dst: Reg,
        /// Source register (tensor).
        src: Reg,
        /// Copy direction: 0=H2D, 1=D2H, 2=D2D.
        direction: u8,
    },
    /// GPU memory allocation.
    GpuAlloc {
        /// Destination register for allocated tensor.
        dst: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Device ID register.
        device: Reg,
    },

    // ========================================================================
    // GPU Extended - Streams
    // ========================================================================
    /// Create new GPU stream.
    GpuStreamCreate {
        /// Destination register for stream handle.
        dst: Reg,
    },
    /// Create GPU stream with priority.
    GpuStreamCreateWithPriority {
        /// Destination register for stream handle.
        dst: Reg,
        /// Priority register (lower = higher priority).
        priority: Reg,
    },
    /// Create non-blocking GPU stream.
    GpuStreamCreateNonBlocking {
        /// Destination register for stream handle.
        dst: Reg,
    },
    /// Destroy GPU stream.
    GpuStreamDestroy {
        /// Stream handle register.
        stream: Reg,
    },
    /// Query stream completion status (non-blocking).
    GpuStreamQuery {
        /// Destination register for status (1=complete, 0=executing).
        dst: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Make stream wait for event.
    GpuStreamWaitEvent {
        /// Stream handle register.
        stream: Reg,
        /// Event handle register.
        event: Reg,
    },
    /// Get stream priority.
    GpuStreamGetPriority {
        /// Destination register for priority value.
        dst: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Add callback to stream (called when stream operations complete).
    GpuStreamAddCallback {
        /// Stream handle register.
        stream: Reg,
        /// Callback function table index.
        callback_id: u32,
        /// User data register.
        user_data: Reg,
    },

    // ========================================================================
    // GPU Extended - Events
    // ========================================================================
    /// Create GPU event.
    GpuEventCreate {
        /// Destination register for event handle.
        dst: Reg,
    },
    /// Create GPU event with flags.
    GpuEventCreateWithFlags {
        /// Destination register for event handle.
        dst: Reg,
        /// Flags: 0x01=BlockingSync, 0x02=DisableTiming, 0x04=Interprocess.
        flags: u8,
    },
    /// Destroy GPU event.
    GpuEventDestroy {
        /// Event handle register.
        event: Reg,
    },
    /// Record event on stream.
    GpuEventRecord {
        /// Event handle register.
        event: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Record event with flags.
    GpuEventRecordWithFlags {
        /// Event handle register.
        event: Reg,
        /// Stream handle register.
        stream: Reg,
        /// Flags register.
        flags: u8,
    },
    /// Synchronize on event (blocking).
    GpuEventSynchronize {
        /// Event handle register.
        event: Reg,
    },
    /// Query event status (non-blocking).
    GpuEventQuery {
        /// Destination register for status (1=completed, 0=pending).
        dst: Reg,
        /// Event handle register.
        event: Reg,
    },
    /// Compute elapsed time between events (milliseconds).
    GpuEventElapsed {
        /// Destination register for elapsed time (Float).
        dst: Reg,
        /// Start event register.
        start_event: Reg,
        /// End event register.
        end_event: Reg,
    },

    // ========================================================================
    // GPU Extended - Device Management
    // ========================================================================
    /// Get current device ID.
    GpuGetDevice {
        /// Destination register for device ID.
        dst: Reg,
    },
    /// Set current device.
    GpuSetDevice {
        /// Device ID register.
        device: Reg,
    },
    /// Get device count.
    GpuGetDeviceCount {
        /// Destination register for count.
        dst: Reg,
    },
    /// Get device property.
    GpuGetDeviceProperty {
        /// Destination register for property value.
        dst: Reg,
        /// Device ID register.
        device: Reg,
        /// Property ID: 0=name, 1=compute_cap, 2=multiprocessors, 3=max_threads,
        /// 4=warp_size, 5=global_mem, 6=shared_mem, 7=max_blocks.
        property_id: u8,
    },
    /// Get device memory info.
    GpuGetMemoryInfo {
        /// Destination register for free bytes.
        free: Reg,
        /// Destination register for total bytes.
        total: Reg,
        /// Device ID register.
        device: Reg,
    },
    /// Check if device can access peer memory.
    GpuCanAccessPeer {
        /// Destination register for result (1=can access, 0=cannot).
        dst: Reg,
        /// Device ID register.
        device: Reg,
        /// Peer device ID register.
        peer_device: Reg,
    },
    /// Enable peer memory access between devices.
    GpuEnablePeerAccess {
        /// Device ID register.
        device: Reg,
        /// Peer device ID register.
        peer_device: Reg,
    },
    /// Disable peer memory access.
    GpuDisablePeerAccess {
        /// Device ID register.
        device: Reg,
        /// Peer device ID register.
        peer_device: Reg,
    },
    /// Reset device (free all allocations).
    GpuDeviceReset {
        /// Device ID register.
        device: Reg,
    },
    /// Set device flags.
    GpuSetDeviceFlags {
        /// Flags: 0x01=ScheduleSpin, 0x02=ScheduleYield, 0x04=ScheduleBlocking.
        flags: u8,
    },

    // ========================================================================
    // GPU Extended - Memory Operations
    // ========================================================================
    /// Asynchronous memory copy.
    GpuMemcpyAsync {
        /// Destination register (tensor).
        dst: Reg,
        /// Source register (tensor).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Copy direction: 0=H2D, 1=D2H, 2=D2D, 3=H2H.
        direction: u8,
        /// Stream handle register.
        stream: Reg,
    },
    /// Free GPU memory.
    GpuFree {
        /// Pointer register.
        ptr: Reg,
    },
    /// Pin host memory for faster transfers.
    GpuPinMemory {
        /// Pointer register.
        ptr: Reg,
        /// Size in bytes register.
        size: Reg,
    },
    /// Unpin previously pinned host memory.
    GpuUnpinMemory {
        /// Pointer register.
        ptr: Reg,
    },
    /// Prefetch memory to device.
    GpuPrefetch {
        /// Pointer register.
        ptr: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Device ID register.
        device: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Set memory to value (synchronous).
    GpuMemset {
        /// Pointer register.
        ptr: Reg,
        /// Value to set.
        value: u8,
        /// Size in bytes register.
        size: Reg,
    },
    /// Set memory to value (asynchronous).
    GpuMemsetAsync {
        /// Pointer register.
        ptr: Reg,
        /// Value to set.
        value: u8,
        /// Size in bytes register.
        size: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// 2D memory copy for pitched allocations.
    GpuMemcpy2D {
        /// Destination register.
        dst: Reg,
        /// Destination pitch register.
        dst_pitch: Reg,
        /// Source register.
        src: Reg,
        /// Source pitch register.
        src_pitch: Reg,
        /// Width register.
        width: Reg,
        /// Height register.
        height: Reg,
        /// Copy direction: 0=H2D, 1=D2H, 2=D2D, 3=H2H.
        direction: u8,
    },
    /// 2D async memory copy.
    GpuMemcpy2DAsync {
        /// Destination register.
        dst: Reg,
        /// Destination pitch register.
        dst_pitch: Reg,
        /// Source register.
        src: Reg,
        /// Source pitch register.
        src_pitch: Reg,
        /// Width register.
        width: Reg,
        /// Height register.
        height: Reg,
        /// Copy direction: 0=H2D, 1=D2H, 2=D2D, 3=H2H.
        direction: u8,
        /// Stream handle register.
        stream: Reg,
    },

    /// Host-to-device memory copy (synchronous, direction-specific).
    GpuMemcpyH2D {
        /// Destination register (device pointer).
        dst: Reg,
        /// Source register (host pointer).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
    },

    /// Device-to-host memory copy (synchronous, direction-specific).
    GpuMemcpyD2H {
        /// Destination register (host pointer).
        dst: Reg,
        /// Source register (device pointer).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
    },

    /// Device-to-device memory copy (synchronous, direction-specific).
    GpuMemcpyD2D {
        /// Destination register (device pointer).
        dst: Reg,
        /// Source register (device pointer).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
    },

    /// Host-to-device async memory copy (direction-specific).
    GpuMemcpyAsyncH2D {
        /// Destination register (device pointer).
        dst: Reg,
        /// Source register (host pointer).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Stream handle register.
        stream: Reg,
    },

    /// Device-to-host async memory copy (direction-specific).
    GpuMemcpyAsyncD2H {
        /// Destination register (host pointer).
        dst: Reg,
        /// Source register (device pointer).
        src: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Stream handle register.
        stream: Reg,
    },

    // ========================================================================
    // GPU Extended - Unified Memory
    // ========================================================================
    /// Allocate managed memory (accessible from host and device).
    GpuMallocManaged {
        /// Destination register for pointer.
        dst: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Attach flags: 0=Global, 1=Host.
        attach_flags: u8,
    },
    /// Set memory advice for managed memory.
    GpuMemAdvise {
        /// Pointer register.
        ptr: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Advice type: 0=SetReadMostly, 1=UnsetReadMostly, 2=SetPreferredLocation,
        /// 3=UnsetPreferredLocation, 4=SetAccessedBy, 5=UnsetAccessedBy.
        advice: u8,
        /// Device ID register.
        device: Reg,
    },
    /// Asynchronous prefetch for managed memory.
    GpuPrefetchAsync {
        /// Pointer register.
        ptr: Reg,
        /// Size in bytes register.
        size: Reg,
        /// Device ID register.
        device: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Get memory attribute for managed memory.
    GpuMemGetAttribute {
        /// Destination register for attribute value.
        dst: Reg,
        /// Pointer register.
        ptr: Reg,
        /// Attribute ID: 0=Type, 1=Device, 2=BaseAddr, 3=Size.
        attribute: u8,
    },

    // ========================================================================
    // GPU Extended - CUDA Graphs / Metal ICB
    // ========================================================================
    /// Create new graph.
    GpuGraphCreate {
        /// Destination register for graph handle.
        dst: Reg,
    },
    /// Begin capturing operations into graph.
    GpuGraphBeginCapture {
        /// Stream handle register (operations on this stream are captured).
        stream: Reg,
        /// Capture mode: 0=Global, 1=ThreadLocal, 2=Relaxed.
        mode: u8,
    },
    /// End capturing and produce graph.
    GpuGraphEndCapture {
        /// Destination register for graph handle.
        dst: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Instantiate graph into executable form.
    GpuGraphInstantiate {
        /// Destination register for graph exec handle.
        dst: Reg,
        /// Graph handle register.
        graph: Reg,
    },
    /// Launch instantiated graph on stream.
    GpuGraphLaunch {
        /// Graph exec handle register.
        graph_exec: Reg,
        /// Stream handle register.
        stream: Reg,
    },
    /// Destroy graph.
    GpuGraphDestroy {
        /// Graph handle register.
        graph: Reg,
    },
    /// Destroy instantiated graph.
    GpuGraphExecDestroy {
        /// Graph exec handle register.
        graph_exec: Reg,
    },
    /// Update graph exec with new graph.
    GpuGraphExecUpdate {
        /// Graph exec handle register.
        graph_exec: Reg,
        /// New graph handle register.
        graph: Reg,
    },

    // ========================================================================
    // GPU Extended - Profiling
    // ========================================================================
    /// Start profiling range.
    GpuProfileRangeStart {
        /// Range name constant pool index.
        name_id: u32,
    },
    /// End profiling range.
    GpuProfileRangeEnd,
    /// Push profiling marker.
    GpuProfileMarkerPush {
        /// Marker name constant pool index.
        name_id: u32,
    },
    /// Pop profiling marker.
    GpuProfileMarkerPop,

    // ========================================================================
    // GPU Extended - Device Enumeration
    // ========================================================================
    /// Enumerate available GPU devices for a specific backend.
    GpuEnumerateDevices {
        /// Destination register for device list.
        dst: Reg,
        /// Backend type: 0=CUDA, 1=Metal, 2=ROCm, 3=Vulkan.
        backend: u8,
    },

    // ========================================================================
    // GPU Extended - Kernel Execution (Advanced)
    // ========================================================================
    /// Launch cooperative kernel (grid-wide synchronization).
    GpuLaunchCooperative {
        /// Kernel function table index.
        kernel_id: u32,
        /// Grid dimensions (x, y, z) registers.
        grid: [Reg; 3],
        /// Block dimensions (x, y, z) registers.
        block: [Reg; 3],
        /// Shared memory size register.
        shared_mem: Reg,
        /// Stream handle register.
        stream: Reg,
        /// Kernel argument registers.
        args: Vec<Reg>,
    },
    /// Launch kernel across multiple devices.
    GpuLaunchMultiDevice {
        /// Kernel function table index.
        kernel_id: u32,
        /// Device IDs register (array).
        devices: Reg,
        /// Grid dimensions (x, y, z) registers.
        grid: [Reg; 3],
        /// Block dimensions (x, y, z) registers.
        block: [Reg; 3],
        /// Shared memory size register.
        shared_mem: Reg,
        /// Kernel argument registers.
        args: Vec<Reg>,
    },
    /// Synchronize device (block until all operations complete).
    GpuDeviceSync,

    // ========================================================================
    // Additional Tensor Operations
    // ========================================================================
    /// Create tensor filled with constant value.
    TensorFull {
        /// Destination register.
        dst: Reg,
        /// Fill value register.
        value: Reg,
        /// Shape registers.
        shape: Vec<Reg>,
        /// Data type.
        dtype: TensorDType,
    },
    /// Create tensor from slice of values.
    TensorFromSlice {
        /// Destination register.
        dst: Reg,
        /// Source data register (list or array).
        data: Reg,
        /// Shape registers.
        shape: Vec<Reg>,
        /// Data type.
        dtype: TensorDType,
    },
    /// Create range tensor [start, end) with step.
    TensorArange {
        /// Destination register.
        dst: Reg,
        /// Start value register.
        start: Reg,
        /// End value register.
        end: Reg,
        /// Step value register.
        step: Reg,
        /// Data type.
        dtype: TensorDType,
    },
    /// Create linearly-spaced tensor.
    TensorLinspace {
        /// Destination register.
        dst: Reg,
        /// Start value register.
        start: Reg,
        /// End value register.
        end: Reg,
        /// Number of points register.
        num: Reg,
        /// Data type.
        dtype: TensorDType,
    },
    /// Create random tensor with uniform distribution [0, 1).
    TensorRand {
        /// Destination register.
        dst: Reg,
        /// Shape registers.
        shape: Vec<Reg>,
        /// Data type.
        dtype: TensorDType,
    },
    /// Deep clone tensor.
    TensorClone {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
    },
    /// Create identity matrix.
    TensorIdentity {
        /// Destination register.
        dst: Reg,
        /// Size register (n for n×n matrix).
        size: Reg,
        /// Data type.
        dtype: TensorDType,
    },
    /// Reshape tensor (view if contiguous).
    TensorReshape {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// New shape registers.
        shape: Vec<Reg>,
    },
    /// Transpose tensor axes.
    TensorTranspose {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Permutation of axes (empty = reverse).
        perm: Vec<u8>,
    },
    /// Slice tensor along dimensions.
    TensorSlice {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Start indices registers.
        starts: Vec<Reg>,
        /// End indices registers.
        ends: Vec<Reg>,
    },
    /// Index selection.
    TensorIndex {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Index tensor register.
        indices: Reg,
        /// Axis to index along.
        axis: u8,
    },
    /// Concatenate tensors along axis.
    TensorConcat {
        /// Destination register.
        dst: Reg,
        /// Source tensor registers.
        tensors: Vec<Reg>,
        /// Axis to concatenate along.
        axis: u8,
    },
    /// Stack tensors along new axis.
    TensorStack {
        /// Destination register.
        dst: Reg,
        /// Source tensor registers.
        tensors: Vec<Reg>,
        /// Axis to stack along.
        axis: u8,
    },
    /// Broadcast tensor to shape.
    TensorBroadcast {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Target shape registers.
        shape: Vec<Reg>,
    },
    /// Remove size-1 dimensions.
    TensorSqueeze {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axes to squeeze (empty = all).
        axes: Vec<u8>,
    },
    /// Element-wise comparison.
    TensorCmp {
        /// Comparison operation.
        op: CompareOp,
        /// Destination register (bool tensor).
        dst: Reg,
        /// Left operand register.
        a: Reg,
        /// Right operand register.
        b: Reg,
    },
    /// Conditional selection: where(cond, x, y).
    TensorWhere {
        /// Destination register.
        dst: Reg,
        /// Condition tensor register.
        cond: Reg,
        /// True-branch tensor register.
        x: Reg,
        /// False-branch tensor register.
        y: Reg,
    },
    /// Clamp values to [min, max].
    TensorClamp {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Minimum value register.
        min: Reg,
        /// Maximum value register.
        max: Reg,
    },
    /// Type cast.
    TensorCast {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Target data type.
        dtype: TensorDType,
    },
    /// Masked fill: fill where mask is true.
    TensorMaskedFill {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Mask tensor register (bool).
        mask: Reg,
        /// Fill value register.
        value: Reg,
    },
    /// Linear interpolation: a + t * (b - a).
    TensorLerp {
        /// Destination register.
        dst: Reg,
        /// Start tensor register.
        a: Reg,
        /// End tensor register.
        b: Reg,
        /// Interpolation factor register.
        t: Reg,
    },
    /// Tensor dot product along axes.
    TensorDot {
        /// Destination register.
        dst: Reg,
        /// Left tensor register.
        a: Reg,
        /// Right tensor register.
        b: Reg,
        /// Axes of a to sum over.
        axes_a: Vec<u8>,
        /// Axes of b to sum over.
        axes_b: Vec<u8>,
    },
    /// Convolution (1D/2D/3D).
    TensorConv {
        /// Destination register.
        dst: Reg,
        /// Input tensor register.
        input: Reg,
        /// Kernel tensor register.
        kernel: Reg,
        /// Optional bias register.
        bias: Option<Reg>,
        /// Stride (per dimension).
        stride: Vec<u8>,
        /// Padding (per dimension).
        padding: Vec<u8>,
        /// Dilation (per dimension).
        dilation: Vec<u8>,
        /// Number of groups.
        groups: u8,
    },
    /// Batched matrix multiplication.
    TensorBatchMatmul {
        /// Destination register.
        dst: Reg,
        /// Left tensor register [..., M, K].
        a: Reg,
        /// Right tensor register [..., K, N].
        b: Reg,
    },
    /// Einstein summation.
    TensorEinsum {
        /// Destination register.
        dst: Reg,
        /// Input tensor registers.
        inputs: Vec<Reg>,
        /// Einsum equation string index in constant pool.
        equation_id: u32,
    },
    /// Outer product.
    TensorOuter {
        /// Destination register.
        dst: Reg,
        /// Left vector register.
        a: Reg,
        /// Right vector register.
        b: Reg,
    },
    /// Triangular solve: solve A @ x = b.
    TensorTriSolve {
        /// Destination register (x).
        dst: Reg,
        /// Matrix A register (triangular).
        a: Reg,
        /// Vector/matrix b register.
        b: Reg,
        /// True = upper triangular, False = lower.
        upper: bool,
    },
    /// Cholesky decomposition.
    TensorCholesky {
        /// Destination register (L or L^T).
        dst: Reg,
        /// Input symmetric positive-definite matrix register.
        src: Reg,
        /// True = upper triangular result, False = lower.
        upper: bool,
    },
    /// Argmax along axis.
    TensorArgmax {
        /// Destination register (index tensor).
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axis to reduce (-1 = all).
        axis: i8,
        /// Keep dimension as size 1.
        keepdim: bool,
    },
    /// Top-k values and indices.
    TensorTopk {
        /// Destination values register.
        values: Reg,
        /// Destination indices register.
        indices: Reg,
        /// Source tensor register.
        src: Reg,
        /// Number of top elements.
        k: Reg,
        /// Axis to find top-k along.
        axis: i8,
        /// True = largest, False = smallest.
        largest: bool,
    },
    /// Cumulative operation (cumsum, cumprod).
    TensorCumulative {
        /// Cumulative operation type.
        op: TensorCumulativeOp,
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axis to accumulate along.
        axis: i8,
    },
    /// Softmax along axis.
    TensorSoftmax {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axis for softmax (-1 = last).
        axis: i8,
    },
    /// Layer normalization.
    TensorLayerNorm {
        /// Destination register.
        dst: Reg,
        /// Input tensor register.
        input: Reg,
        /// Optional gamma (scale) register.
        gamma: Option<Reg>,
        /// Optional beta (bias) register.
        beta: Option<Reg>,
        /// Normalized shape size (last N dims).
        normalized_shape: u32,
        /// Epsilon for numerical stability.
        eps: f32,
    },
    /// Batch normalization.
    TensorBatchNorm {
        /// Destination register.
        dst: Reg,
        /// Input tensor register.
        input: Reg,
        /// Optional gamma (scale) register.
        gamma: Option<Reg>,
        /// Optional beta (bias) register.
        beta: Option<Reg>,
        /// Running mean register.
        running_mean: Option<Reg>,
        /// Running variance register.
        running_var: Option<Reg>,
        /// Training mode flag.
        training: bool,
        /// Momentum for running stats.
        momentum: f32,
        /// Epsilon for numerical stability.
        eps: f32,
    },
    /// RMS normalization.
    TensorRmsNorm {
        /// Destination register.
        dst: Reg,
        /// Input tensor register.
        input: Reg,
        /// Optional gamma (scale) register.
        gamma: Option<Reg>,
        /// Epsilon for numerical stability.
        eps: f32,
    },
    /// Fast Fourier Transform.
    TensorFft {
        /// Destination register (complex tensor).
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// FFT dimension (-1 = last).
        dim: i8,
        /// True = inverse FFT.
        inverse: bool,
    },
    /// Scatter operation.
    TensorScatter {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Index tensor register.
        index: Reg,
        /// Values tensor register.
        values: Reg,
        /// Axis for scatter.
        axis: i8,
        /// Scatter mode: 0=update, 1=add, 2=mul.
        mode: u8,
    },
    /// Pooling operation (max, avg).
    TensorPool {
        /// Pooling operation type.
        op: TensorPoolOp,
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Kernel size per dimension.
        kernel_size: Vec<u8>,
        /// Stride per dimension.
        stride: Vec<u8>,
        /// Padding per dimension.
        padding: Vec<u8>,
    },

    // ========================================================================
    // TensorExtended Operations (via TensorSubOpcode)
    // ========================================================================

    /// Argmin along axis.
    TensorArgmin {
        /// Destination register (index tensor).
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Axis to reduce (-1 = all).
        axis: i8,
        /// Keep reduced dimension.
        keepdim: bool,
    },

    /// General linear system solve: A @ x = B.
    TensorSolve {
        /// Destination register (solution tensor).
        dst: Reg,
        /// Matrix A register.
        a: Reg,
        /// Matrix B register.
        b: Reg,
    },

    /// Gather elements along axis using indices.
    TensorGather {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Index tensor register.
        index: Reg,
        /// Axis for gather operation.
        axis: i8,
    },

    /// General axis permutation.
    TensorPermute {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// New axis order.
        axes: Vec<u8>,
    },

    /// QR decomposition.
    TensorQR {
        /// Q matrix register.
        q: Reg,
        /// R matrix register.
        r: Reg,
        /// Source matrix register.
        src: Reg,
        /// Mode: 0=reduced, 1=complete, 2=r_only.
        mode: u8,
    },

    /// Singular Value Decomposition.
    TensorSVD {
        /// U matrix register.
        u: Reg,
        /// Singular values register.
        s: Reg,
        /// Vh matrix register.
        vh: Reg,
        /// Source matrix register.
        src: Reg,
        /// Compute full matrices.
        full_matrices: bool,
        /// Compute U and Vh.
        compute_uv: bool,
    },

    /// LU decomposition with pivoting.
    TensorLU {
        /// Permutation matrix register.
        p: Reg,
        /// L matrix register.
        l: Reg,
        /// U matrix register.
        u: Reg,
        /// Source matrix register.
        src: Reg,
    },

    /// Eigenvalue decomposition (general).
    TensorEig {
        /// Eigenvalues register.
        eigenvalues: Reg,
        /// Eigenvectors register.
        eigenvectors: Reg,
        /// Source matrix register.
        src: Reg,
        /// Compute eigenvectors.
        compute_v: bool,
    },

    /// Symmetric eigenvalue decomposition.
    TensorEigSymmetric {
        /// Eigenvalues register.
        eigenvalues: Reg,
        /// Eigenvectors register.
        eigenvectors: Reg,
        /// Source matrix register.
        src: Reg,
        /// Use upper triangle.
        upper: bool,
    },

    /// Least squares solve.
    TensorLstsq {
        /// Solution register.
        x: Reg,
        /// Residuals register.
        residuals: Reg,
        /// Rank register.
        rank: Reg,
        /// Singular values register.
        s: Reg,
        /// Matrix A register.
        a: Reg,
        /// Matrix B register.
        b: Reg,
        /// Reciprocal condition number cutoff.
        rcond: f64,
    },

    /// Matrix determinant.
    TensorDet {
        /// Destination register.
        dst: Reg,
        /// Source matrix register.
        src: Reg,
    },

    /// Matrix trace.
    TensorTrace {
        /// Destination register.
        dst: Reg,
        /// Source matrix register.
        src: Reg,
    },

    /// Matrix/vector norm.
    TensorNorm {
        /// Destination register.
        dst: Reg,
        /// Source tensor register.
        src: Reg,
        /// Norm order: -2=min singular, -1=inf, 0=Frobenius, 1=1-norm, 2=2-norm.
        ord: i8,
    },

    // ========================================================================
    // V-LLSI System Operations
    // ========================================================================

    /// Raw Linux syscall: `dst = syscall(num, a1, a2, a3, a4, a5, a6)`
    /// In VBC interpreter: dispatches to libc syscall
    /// In AOT: compiles to inline syscall instruction
    SyscallLinux {
        /// Destination register for syscall return value.
        dst: Reg,
        /// Syscall number register.
        num: Reg,
        /// Argument 1 register.
        a1: Reg,
        /// Argument 2 register.
        a2: Reg,
        /// Argument 3 register.
        a3: Reg,
        /// Argument 4 register.
        a4: Reg,
        /// Argument 5 register.
        a5: Reg,
        /// Argument 6 register.
        a6: Reg,
    },

    /// Memory map region: `dst = mmap(addr, len, prot, flags, fd, offset)`
    /// Returns pointer to mapped region or error code.
    Mmap {
        /// Destination register for mapped address or error.
        dst: Reg,
        /// Address hint register (0 for kernel choice).
        addr: Reg,
        /// Length register.
        len: Reg,
        /// Protection flags register (PROT_READ, PROT_WRITE, PROT_EXEC).
        prot: Reg,
        /// Mapping flags register (MAP_PRIVATE, MAP_ANONYMOUS, etc.).
        flags: Reg,
        /// File descriptor register (-1 for anonymous).
        fd: Reg,
        /// File offset register.
        offset: Reg,
    },

    /// Unmap memory region: `result = munmap(addr, len)`
    /// Returns 0 on success, -1 on error.
    Munmap {
        /// Destination register for result.
        dst: Reg,
        /// Address of region to unmap.
        addr: Reg,
        /// Length of region to unmap.
        len: Reg,
    },

    /// Atomic load: `dst = atomic_load(ptr, ordering, size)`
    /// Ordering: 0=Relaxed, 1=Acquire, 2=SeqCst
    /// Size: 1=u8, 2=u16, 4=u32, 8=u64
    AtomicLoad {
        /// Destination register.
        dst: Reg,
        /// Pointer to load from.
        ptr: Reg,
        /// Memory ordering (0=Relaxed, 1=Acquire, 2=SeqCst).
        ordering: u8,
        /// Size in bytes (1, 2, 4, or 8).
        size: u8,
    },

    /// Atomic store: `atomic_store(ptr, val, ordering, size)`
    /// Ordering: 0=Relaxed, 1=Release, 2=SeqCst
    /// Size: 1=u8, 2=u16, 4=u32, 8=u64
    AtomicStore {
        /// Pointer to store to.
        ptr: Reg,
        /// Value to store.
        val: Reg,
        /// Memory ordering (0=Relaxed, 1=Release, 2=SeqCst).
        ordering: u8,
        /// Size in bytes (1, 2, 4, or 8).
        size: u8,
    },

    /// Atomic compare-and-swap: `dst = atomic_cas(ptr, expected, desired, ordering, size)`
    /// Returns the original value at ptr. Success if returned == expected.
    /// Size: 1=u8, 2=u16, 4=u32, 8=u64
    AtomicCas {
        /// Destination register (original value).
        dst: Reg,
        /// Pointer to the atomic location.
        ptr: Reg,
        /// Expected value register.
        expected: Reg,
        /// Desired new value register.
        desired: Reg,
        /// Memory ordering (0=Relaxed, 1=Acquire, 2=Release, 3=AcqRel, 4=SeqCst).
        ordering: u8,
        /// Size in bytes (1, 2, 4, or 8).
        size: u8,
    },

    /// Memory fence: `fence(ordering)`
    /// Ordering: 0=Relaxed, 1=Acquire, 2=Release, 3=AcqRel, 4=SeqCst
    AtomicFence {
        /// Memory ordering.
        ordering: u8,
    },

    /// Submit I/O operations to IOEngine: `token = io_submit(engine, ops)`
    /// Returns submission token for later polling.
    IoSubmit {
        /// Destination register for submission token.
        dst: Reg,
        /// IOEngine handle register.
        engine: Reg,
        /// Operations list register.
        ops: Reg,
    },

    /// Poll IOEngine for completions: `results = io_poll(engine, timeout)`
    /// Returns completed operations or empty list if timeout.
    IoPoll {
        /// Destination register for completion results.
        dst: Reg,
        /// IOEngine handle register.
        engine: Reg,
        /// Timeout in nanoseconds register.
        timeout: Reg,
    },

    /// Get thread-local storage: `dst = tls_get(slot)`
    /// Retrieves value from TLS slot.
    TlsGet {
        /// Destination register.
        dst: Reg,
        /// TLS slot index register.
        slot: Reg,
    },

    /// Set thread-local storage: `tls_set(slot, val)`
    /// Stores value into TLS slot.
    TlsSet {
        /// TLS slot index register.
        slot: Reg,
        /// Value to store register.
        val: Reg,
    },

    // ========================================================================
    // Arithmetic Extended
    // ========================================================================
    /// Arithmetic extended operations.
    ///
    /// Uses sub-opcodes for different arithmetic operations:
    /// - Checked arithmetic (CheckedAddI, CheckedSubI, etc.)
    /// - Overflowing arithmetic (OverflowingAddI, OverflowingSubI, etc.)
    /// - Polymorphic arithmetic (PolyAdd, PolySub, etc.)
    ArithExtended {
        /// Arithmetic sub-opcode.
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Tensor Extended
    // ========================================================================
    /// Tensor extended operations.
    ///
    /// Uses sub-opcodes for advanced tensor operations:
    /// - Matrix decompositions (QR, SVD, LU, Eig, etc.)
    /// - Linear algebra (Solve, TriSolve, Inverse, etc.)
    /// - Matrix properties (Det, Rank, Cond, Norm, Trace)
    TensorExtended {
        /// Tensor sub-opcode.
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Math Extended
    // ========================================================================
    /// Math extended operations for transcendental and special functions.
    ///
    /// Uses MathSubOpcode for zero-cost dispatch (~2ns) of 72+ math operations:
    /// - Trigonometric (sin, cos, tan, asin, acos, atan, atan2)
    /// - Hyperbolic (sinh, cosh, tanh, asinh, acosh, atanh)
    /// - Exponential/Logarithmic (exp, exp2, expm1, log, log2, log10, log1p, pow, powi)
    /// - Root/Power (sqrt, cbrt, hypot)
    /// - Rounding (floor, ceil, round, trunc)
    /// - Special (abs, copysign, fma, fmod, remainder, fdim, minnum, maxnum)
    /// - Classification (is_nan, is_infinite, is_finite)
    ///
    /// All operations map 1:1 to LLVM intrinsics for AOT compilation.
    MathExtended {
        /// Math sub-opcode (MathSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // SIMD Extended
    // ========================================================================
    /// SIMD extended operations for platform-agnostic vector operations.
    ///
    /// Uses SimdSubOpcode for zero-cost dispatch (~2ns) of SIMD operations:
    /// - Arithmetic (Add, Sub, Mul, Div, FMA)
    /// - Comparison (Min, Max, Eq, Lt, Le, Gt, Ge)
    /// - Data movement (Splat, Broadcast, Shuffle, Extract, Insert)
    /// - Reductions (ReduceAdd, ReduceMul, ReduceMin, ReduceMax)
    /// - Type conversion (CvtIntToFloat, CvtFloatToInt)
    ///
    /// Provides fallback scalar implementations for portability.
    SimdExtended {
        /// SIMD sub-opcode (SimdSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Char Extended
    // ========================================================================
    /// Character extended operations for classification and conversion.
    ///
    /// Uses CharSubOpcode for zero-cost dispatch (~2ns) of character operations:
    /// - ASCII classification (IsAlphabetic, IsDigit, IsAlphanumeric, IsWhitespace, etc.)
    /// - Unicode classification (IsAlphabeticUnicode, IsNumericUnicode, etc.)
    /// - Case conversion (ToUpper, ToLower, ToTitlecase)
    /// - Character properties (IsControl, IsPunctuation, IsSymbol)
    ///
    /// Optimized for ASCII with Unicode fallbacks.
    CharExtended {
        /// Char sub-opcode (CharSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // CBGR Extended
    // ========================================================================
    /// CBGR (Capability-Based Generational References) extended operations.
    ///
    /// Uses CbgrSubOpcode for zero-cost dispatch (~2ns) of memory safety operations:
    /// - Reference creation (RefSlice, RefInterior, RefProject)
    /// - Capability manipulation (CapAttenuate, CapTransfer, CapRevoke)
    /// - Generation tracking (GenCheck, GenInvalidate)
    /// - Slice operations (SliceLen, Unslice, SplitAt)
    /// - Lifetime/epoch management (EpochBegin, EpochEnd)
    ///
    /// Core primitive for Verum's memory safety model.
    CbgrExtended {
        /// CBGR sub-opcode (CbgrSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Text Extended
    // ========================================================================
    /// Text extended operations.
    ///
    /// Uses TextSubOpcode for zero-cost dispatch (~2ns) of text operations:
    /// - Text creation (FromStatic)
    /// - Parsing (ParseInt, ParseFloat)
    /// - Conversion (IntToText, FloatToText)
    /// - Properties (ByteLen, CharLen, IsEmpty, IsUtf8)
    ///
    /// Opcode: 0x79 (TextExtended)
    TextExtended {
        /// Text sub-opcode (TextSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Cubical Extended
    // ========================================================================
    /// Cubical type theory extended operations.
    ///
    /// Uses CubicalSubOpcode for cubical type theory operations:
    /// - Path construction (PathRefl, PathLambda, PathApp, PathSym, PathTrans, PathAp)
    /// - Transport and composition (Transport, Hcomp)
    /// - Interval operations (IntervalI0, IntervalI1, IntervalMeet, IntervalJoin, IntervalRev)
    /// - Univalence (Ua, UaInv, EquivFwd, EquivBwd)
    CubicalExtended {
        /// Cubical sub-opcode (CubicalSubOpcode as u8).
        sub_op: u8,
        /// Destination register.
        dst: Reg,
        /// Argument registers.
        args: Vec<Reg>,
    },

    // ========================================================================
    // GPU Extended
    // ========================================================================
    /// GPU extended operations.
    ///
    /// Uses sub-opcodes for GPU operations:
    /// - Device management (GetDevice, SetDevice, DeviceReset)
    /// - Memory (Allocate, Free, CopyHostToDevice, CopyDeviceToHost)
    /// - Streams (StreamCreate, StreamDestroy, StreamSynchronize)
    /// - Kernel launch (LaunchKernel)
    GpuExtended {
        /// GPU sub-opcode.
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // FFI Extended
    // ========================================================================
    /// FFI extended operations.
    ///
    /// Uses sub-opcodes for different FFI operations:
    /// - LoadSymbol, CallFfiC, CallFfiStdcall, etc.
    FfiExtended {
        /// FFI sub-opcode.
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Generic Extended (#167 Part A)
    // ========================================================================
    /// General-purpose extension instruction.
    ///
    /// Encoded as `[0x1F (Opcode::Extended)] [sub_op:u8] [operand_len:u8]
    /// [operands...]`.  The `sub_op` byte selects the extended-instruction
    /// kind from the 256-entry sub-op space carved out of what was
    /// previously the reserved `IntArith1F` opcode slot.  This is the
    /// home for new first-class instructions that don't fit any
    /// existing extension namespace (Math/Tensor/Cbgr/Ffi/etc.).
    ///
    /// Defined sub-ops:
    /// - 0x00 — reserved no-op (forward-compat anchor; encoder must
    ///          never emit, decoder accepts and skips its operands).
    ///
    /// Future sub-ops (#167 Part B + later work) will land here as
    /// they're wired through codegen / interpreter / LLVM.
    Extended {
        /// Extension sub-opcode (`ExtendedSubOpcode`).
        sub_op: u8,
        /// Operand bytes (length-prefixed; decoded by handler).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Memory Extended
    // ========================================================================
    /// Memory extended operations for heap allocation.
    ///
    /// Uses sub-opcodes for different memory operations:
    /// - 0x00: alloc - allocate heap memory
    /// - 0x01: alloc_zeroed - allocate zeroed heap memory
    /// - 0x02: dealloc - deallocate heap memory
    /// - 0x03: realloc - reallocate heap memory
    /// - 0x04: swap - swap two values in place
    /// - 0x05: replace - replace value and return old
    MemExtended {
        /// Memory sub-opcode.
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    // ========================================================================
    // Logging Extended
    // ========================================================================
    /// Logging extended operations.
    ///
    /// Uses LogSubOpcode for structured logging:
    /// - Info, Warning, Error, Debug levels
    ///
    /// Opcode: 0xBE (LogExtended)
    LogExtended {
        /// Log sub-opcode (LogSubOpcode as u8).
        sub_op: u8,
        /// Operand bytes (decoded by interpreter).
        operands: Vec<u8>,
    },

    /// Loop optimization hint marker. No-op for interpreter.
    /// LLVM codegen reads this to attach loop metadata.
    LoopHint {
        /// Loop hints (unroll, vectorize).
        hints: crate::module::LoopHints,
    },

    /// Branch likelihood hint. No-op for interpreter.
    /// LLVM codegen uses this for branch weight metadata.
    BranchHint {
        /// true = @likely, false = @unlikely
        likely: bool,
    },

    /// Prefetch hint. No-op for interpreter.
    /// LLVM codegen emits `llvm.prefetch` intrinsic.
    PrefetchHint {
        /// Address register to prefetch.
        addr: Reg,
        /// true = read, false = write
        is_read: bool,
        /// Locality hint 0-3
        locality: u8,
    },

    /// Optimization barrier. Prevents optimizations across this point.
    /// LLVM codegen emits an inline asm barrier.
    OptBarrier {
        /// Register whose value must not be optimized away.
        reg: Reg,
    },

    /// Raw bytecode (for opcodes not yet decoded).
    Raw {
        /// Opcode byte value.
        opcode: Opcode,
        /// Raw operand bytes.
        data: Vec<u8>,
    },
}

// ============================================================================
// Supporting Enums for Instructions
// ============================================================================

/// Binary integer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryIntOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Modulo.
    Mod,
    /// Power.
    Pow,
}

/// Binary float operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryFloatOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Power.
    Pow,
    /// Modulo.
    Mod,
}

/// Binary generic operations (via protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryGenericOp {
    /// Add protocol.
    Add,
    /// Sub protocol.
    Sub,
    /// Mul protocol.
    Mul,
    /// Div protocol.
    Div,
}

/// Float to Int conversion modes.
///
/// Specifies how floating-point values should be rounded when converting to integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum FloatToIntMode {
    /// Truncate toward zero (default): 3.7 → 3, -3.7 → -3
    #[default]
    Trunc = 0,
    /// Floor (round toward negative infinity): 3.7 → 3, -3.7 → -4
    Floor = 1,
    /// Ceiling (round toward positive infinity): 3.7 → 4, -3.7 → -3
    Ceil = 2,
    /// Round to nearest integer (half away from zero): 3.5 → 4, -3.5 → -4
    Round = 3,
}

impl FloatToIntMode {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Trunc),
            1 => Some(Self::Floor),
            2 => Some(Self::Ceil),
            3 => Some(Self::Round),
            _ => None,
        }
    }

    /// Converts to byte value.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// Unary integer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnaryIntOp {
    /// Negate.
    Neg,
    /// Absolute value.
    Abs,
    /// Increment.
    Inc,
    /// Decrement.
    Dec,
}

/// Unary float operations.
///
/// Sub-opcode encoding for NegF (0x25) instruction:
/// - Basic (0-10): Neg, Abs, Sqrt, Exp, Log, Sin, Cos, Tan, Floor, Ceil, Round
/// - Inverse trig (11-13): Asin, Acos, Atan
/// - Hyperbolic (14-16): Sinh, Cosh, Tanh
/// - Inverse hyperbolic (17-19): Asinh, Acosh, Atanh
/// - Log/exp variants (20-22): Log10, Log2, Exp2
/// - Special (23-29): Cbrt, Expm1, Ln1p, Signum, Trunc, Fract, Recip
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum UnaryFloatOp {
    // Basic operations (0-10)
    /// Negate.
    Neg = 0,
    /// Absolute value.
    Abs = 1,
    /// Square root.
    Sqrt = 2,
    /// Natural exponential (e^x).
    Exp = 3,
    /// Natural log (ln).
    Log = 4,
    /// Sine.
    Sin = 5,
    /// Cosine.
    Cos = 6,
    /// Tangent.
    Tan = 7,
    /// Floor (round down).
    Floor = 8,
    /// Ceiling (round up).
    Ceil = 9,
    /// Round to nearest.
    Round = 10,

    // Inverse trigonometric (11-13)
    /// Inverse sine (arcsin).
    Asin = 11,
    /// Inverse cosine (arccos).
    Acos = 12,
    /// Inverse tangent (arctan).
    Atan = 13,

    // Hyperbolic (14-16)
    /// Hyperbolic sine.
    Sinh = 14,
    /// Hyperbolic cosine.
    Cosh = 15,
    /// Hyperbolic tangent.
    Tanh = 16,

    // Inverse hyperbolic (17-19)
    /// Inverse hyperbolic sine.
    Asinh = 17,
    /// Inverse hyperbolic cosine.
    Acosh = 18,
    /// Inverse hyperbolic tangent.
    Atanh = 19,

    // Logarithm and exponential variants (20-22)
    /// Base-10 logarithm.
    Log10 = 20,
    /// Base-2 logarithm.
    Log2 = 21,
    /// Base-2 exponential (2^x).
    Exp2 = 22,

    // Special functions (23-29)
    /// Cube root.
    Cbrt = 23,
    /// exp(x) - 1, accurate for small x.
    Expm1 = 24,
    /// ln(1 + x), accurate for small x.
    Ln1p = 25,
    /// Sign function (-1, 0, or 1).
    Signum = 26,
    /// Truncate toward zero.
    Trunc = 27,
    /// Fractional part.
    Fract = 28,
    /// Reciprocal (1/x).
    Recip = 29,
}

/// Bitwise operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BitwiseOp {
    /// Bitwise AND.
    And,
    /// Bitwise OR.
    Or,
    /// Bitwise XOR.
    Xor,
    /// Bitwise NOT (unary, b is ignored).
    Not,
    /// Shift left.
    Shl,
    /// Arithmetic shift right.
    Shr,
    /// Logical shift right.
    Ushr,
}

/// Comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompareOp {
    /// Equal.
    Eq = 0x00,
    /// Not equal.
    Ne = 0x01,
    /// Less than.
    Lt = 0x02,
    /// Less or equal.
    Le = 0x03,
    /// Greater than.
    Gt = 0x04,
    /// Greater or equal.
    Ge = 0x05,
}

impl CompareOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Eq,
            0x01 => Self::Ne,
            0x02 => Self::Lt,
            0x03 => Self::Le,
            0x04 => Self::Gt,
            0x05 => Self::Ge,
            _ => Self::Eq, // Default
        }
    }
}

impl TryFrom<u8> for CompareOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(Self::from_byte(value))
    }
}

/// Gradient mode for autodiff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GradMode {
    /// Reverse-mode (VJP) - efficient for few outputs.
    Reverse,
    /// Forward-mode (JVP) - efficient for few inputs.
    Forward,
    /// Compiler chooses optimal mode.
    Auto,
}

/// Tensor data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorDType {
    /// 64-bit float.
    F64 = 0x00,
    /// 32-bit float.
    F32 = 0x01,
    /// 16-bit float.
    F16 = 0x02,
    /// bfloat16.
    BF16 = 0x03,
    /// 64-bit integer.
    I64 = 0x04,
    /// 32-bit integer.
    I32 = 0x05,
    /// 16-bit integer.
    I16 = 0x06,
    /// 8-bit integer.
    I8 = 0x07,
    /// 64-bit unsigned.
    U64 = 0x08,
    /// 32-bit unsigned.
    U32 = 0x09,
    /// 16-bit unsigned.
    U16 = 0x0A,
    /// 8-bit unsigned.
    U8 = 0x0B,
    /// Boolean.
    Bool = 0x0C,
    /// Complex 64.
    Complex64 = 0x0D,
    /// Complex 128.
    Complex128 = 0x0E,
}

impl TensorDType {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::F64,
            0x01 => Self::F32,
            0x02 => Self::F16,
            0x03 => Self::BF16,
            0x04 => Self::I64,
            0x05 => Self::I32,
            0x06 => Self::I16,
            0x07 => Self::I8,
            0x08 => Self::U64,
            0x09 => Self::U32,
            0x0A => Self::U16,
            0x0B => Self::U8,
            0x0C => Self::Bool,
            0x0D => Self::Complex64,
            0x0E => Self::Complex128,
            _ => Self::F64, // Default
        }
    }
}

/// Tensor binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorBinaryOp {
    /// Element-wise add.
    Add = 0x00,
    /// Element-wise subtract.
    Sub = 0x01,
    /// Element-wise multiply.
    Mul = 0x02,
    /// Element-wise divide.
    Div = 0x03,
    /// Element-wise power.
    Pow = 0x04,
    /// Element-wise modulo.
    Mod = 0x05,
    /// Element-wise min.
    Min = 0x06,
    /// Element-wise max.
    Max = 0x07,
}

impl TensorBinaryOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Add,
            0x01 => Self::Sub,
            0x02 => Self::Mul,
            0x03 => Self::Div,
            0x04 => Self::Pow,
            0x05 => Self::Mod,
            0x06 => Self::Min,
            0x07 => Self::Max,
            _ => Self::Add, // Default
        }
    }
}

/// Tensor unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorUnaryOp {
    /// Negate.
    Neg = 0x00,
    /// Absolute value.
    Abs = 0x01,
    /// Square root.
    Sqrt = 0x02,
    /// Exponential.
    Exp = 0x03,
    /// Natural log.
    Log = 0x04,
    /// Sine.
    Sin = 0x05,
    /// Cosine.
    Cos = 0x06,
    /// Tangent.
    Tan = 0x07,
    /// Hyperbolic tangent.
    Tanh = 0x08,
    /// Sigmoid.
    Sigmoid = 0x09,
    /// ReLU.
    Relu = 0x0A,
    /// GELU.
    Gelu = 0x0B,
    /// SiLU (Swish).
    Silu = 0x0C,
    /// Floor.
    Floor = 0x0D,
    /// Ceiling.
    Ceil = 0x0E,
    /// Round.
    Round = 0x0F,
    /// Sign.
    Sign = 0x10,
    /// Reciprocal sqrt.
    Rsqrt = 0x11,
    /// Error function.
    Erf = 0x12,
    /// Log base 2.
    Log2 = 0x13,
    /// Softplus.
    Softplus = 0x14,
    /// Mish activation.
    Mish = 0x15,
}

impl TensorUnaryOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Neg,
            0x01 => Self::Abs,
            0x02 => Self::Sqrt,
            0x03 => Self::Exp,
            0x04 => Self::Log,
            0x05 => Self::Sin,
            0x06 => Self::Cos,
            0x07 => Self::Tan,
            0x08 => Self::Tanh,
            0x09 => Self::Sigmoid,
            0x0A => Self::Relu,
            0x0B => Self::Gelu,
            0x0C => Self::Silu,
            0x0D => Self::Floor,
            0x0E => Self::Ceil,
            0x0F => Self::Round,
            0x10 => Self::Sign,
            0x11 => Self::Rsqrt,
            0x12 => Self::Erf,
            0x13 => Self::Log2,
            0x14 => Self::Softplus,
            0x15 => Self::Mish,
            _ => Self::Neg, // Default
        }
    }
}

/// Tensor reduction operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TensorReduceOp {
    /// Sum.
    Sum = 0x00,
    /// Product.
    Prod = 0x01,
    /// Maximum.
    Max = 0x02,
    /// Minimum.
    Min = 0x03,
    /// Mean.
    Mean = 0x04,
    /// Variance.
    Var = 0x05,
    /// Standard deviation.
    Std = 0x06,
    /// L2 norm.
    Norm = 0x07,
    /// Log-sum-exp.
    LogSumExp = 0x08,
    /// All (logical and).
    All = 0x09,
    /// Any (logical or).
    Any = 0x0A,
}

impl TensorReduceOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Sum,
            0x01 => Self::Prod,
            0x02 => Self::Max,
            0x03 => Self::Min,
            0x04 => Self::Mean,
            0x05 => Self::Var,
            0x06 => Self::Std,
            0x07 => Self::Norm,
            0x08 => Self::LogSumExp,
            0x09 => Self::All,
            0x0A => Self::Any,
            _ => Self::Sum, // Default
        }
    }
}

/// Cumulative operation type for TensorCumulative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorCumulativeOp {
    /// Cumulative sum.
    Sum = 0x00,
    /// Cumulative product.
    Prod = 0x01,
    /// Cumulative max.
    Max = 0x02,
    /// Cumulative min.
    Min = 0x03,
}

impl TensorCumulativeOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Sum,
            0x01 => Self::Prod,
            0x02 => Self::Max,
            0x03 => Self::Min,
            _ => Self::Sum, // Default
        }
    }
}

/// Pooling operation type for TensorPool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorPoolOp {
    /// Max pooling.
    Max = 0x00,
    /// Average pooling.
    Avg = 0x01,
    /// Sum pooling.
    Sum = 0x02,
    /// Adaptive max pooling.
    AdaptiveMax = 0x03,
    /// Adaptive average pooling.
    AdaptiveAvg = 0x04,
}

impl TensorPoolOp {
    /// Converts from byte value.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Max,
            0x01 => Self::Avg,
            0x02 => Self::Sum,
            0x03 => Self::AdaptiveMax,
            0x04 => Self::AdaptiveAvg,
            _ => Self::Max, // Default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Reg Tests
    // ========================================================================

    #[test]
    fn test_reg_creation() {
        let r0 = Reg::new(0);
        assert_eq!(r0.0, 0);

        let r100 = Reg::new(100);
        assert_eq!(r100.0, 100);

        let r_max = Reg::new(Reg::MAX);
        assert_eq!(r_max.0, Reg::MAX);
    }

    #[test]
    fn test_reg_default() {
        let r: Reg = Default::default();
        assert_eq!(r.0, 0);
    }

    #[test]
    fn test_reg_is_short_boundary() {
        // Short registers: r0-r127
        assert!(Reg(0).is_short());
        assert!(Reg(1).is_short());
        assert!(Reg(63).is_short());
        assert!(Reg(64).is_short());
        assert!(Reg(126).is_short());
        assert!(Reg(127).is_short());

        // Long registers: r128+
        assert!(!Reg(128).is_short());
        assert!(!Reg(129).is_short());
        assert!(!Reg(255).is_short());
        assert!(!Reg(256).is_short());
        assert!(!Reg(1000).is_short());
        assert!(!Reg(16383).is_short());
    }

    #[test]
    fn test_reg_max_value() {
        assert_eq!(Reg::MAX, 16383);
    }

    #[test]
    fn test_reg_equality() {
        let r1 = Reg(42);
        let r2 = Reg(42);
        let r3 = Reg(43);

        assert_eq!(r1, r2);
        assert_ne!(r1, r3);
    }

    #[test]
    fn test_reg_clone_copy() {
        let r1 = Reg(100);
        let r2 = r1; // Copy
        let r3 = r1;

        assert_eq!(r1, r2);
        assert_eq!(r1, r3);
    }

    #[test]
    fn test_reg_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Reg(0));
        set.insert(Reg(1));
        set.insert(Reg(0)); // Duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&Reg(0)));
        assert!(set.contains(&Reg(1)));
    }

    // ========================================================================
    // RegRange Tests
    // ========================================================================

    #[test]
    fn test_reg_range_creation() {
        let range = RegRange::new(Reg(10), 5);
        assert_eq!(range.start, Reg(10));
        assert_eq!(range.count, 5);
    }

    #[test]
    fn test_reg_range_default() {
        let range: RegRange = Default::default();
        assert_eq!(range.start, Reg(0));
        assert_eq!(range.count, 0);
    }

    #[test]
    fn test_reg_range_iter() {
        let range = RegRange::new(Reg(5), 3);
        let regs: Vec<Reg> = range.iter().collect();
        assert_eq!(regs, vec![Reg(5), Reg(6), Reg(7)]);
    }

    #[test]
    fn test_reg_range_iter_empty() {
        let range = RegRange::new(Reg(5), 0);
        let regs: Vec<Reg> = range.iter().collect();
        assert!(regs.is_empty());
    }

    #[test]
    fn test_reg_range_iter_single() {
        let range = RegRange::new(Reg(100), 1);
        let regs: Vec<Reg> = range.iter().collect();
        assert_eq!(regs, vec![Reg(100)]);
    }

    #[test]
    fn test_reg_range_iter_max_count() {
        let range = RegRange::new(Reg(0), 255);
        let regs: Vec<Reg> = range.iter().collect();
        assert_eq!(regs.len(), 255);
        assert_eq!(regs[0], Reg(0));
        assert_eq!(regs[254], Reg(254));
    }

    #[test]
    fn test_reg_range_equality() {
        let r1 = RegRange::new(Reg(5), 3);
        let r2 = RegRange::new(Reg(5), 3);
        let r3 = RegRange::new(Reg(5), 4);
        let r4 = RegRange::new(Reg(6), 3);

        assert_eq!(r1, r2);
        assert_ne!(r1, r3);
        assert_ne!(r1, r4);
    }

    // ========================================================================
    // Opcode Tests
    // ========================================================================

    #[test]
    fn test_opcode_roundtrip() {
        for byte in 0..=255u8 {
            let op = Opcode::from_byte(byte);
            assert_eq!(op.to_byte(), byte);
        }
    }

    #[test]
    fn test_opcode_specific_values() {
        // Data Movement (0x00-0x0F)
        assert_eq!(Opcode::Mov.to_byte(), 0x00);
        assert_eq!(Opcode::LoadK.to_byte(), 0x01);
        assert_eq!(Opcode::LoadI.to_byte(), 0x02);
        assert_eq!(Opcode::LoadF.to_byte(), 0x03);
        assert_eq!(Opcode::LoadTrue.to_byte(), 0x04);
        assert_eq!(Opcode::LoadFalse.to_byte(), 0x05);
        assert_eq!(Opcode::LoadUnit.to_byte(), 0x06);
        assert_eq!(Opcode::LoadT.to_byte(), 0x07);
        assert_eq!(Opcode::LoadSmallI.to_byte(), 0x08);
        assert_eq!(Opcode::LoadNil.to_byte(), 0x09);

        // Integer Arithmetic (0x10-0x1F)
        assert_eq!(Opcode::AddI.to_byte(), 0x10);
        assert_eq!(Opcode::SubI.to_byte(), 0x11);
        assert_eq!(Opcode::MulI.to_byte(), 0x12);
        assert_eq!(Opcode::DivI.to_byte(), 0x13);
        assert_eq!(Opcode::ModI.to_byte(), 0x14);

        // Float Arithmetic (0x20-0x2F)
        assert_eq!(Opcode::AddF.to_byte(), 0x20);
        assert_eq!(Opcode::SubF.to_byte(), 0x21);
        assert_eq!(Opcode::MulF.to_byte(), 0x22);
        assert_eq!(Opcode::DivF.to_byte(), 0x23);

        // Comparison (0x40-0x4F)
        assert_eq!(Opcode::EqI.to_byte(), 0x40);
        assert_eq!(Opcode::NeI.to_byte(), 0x41);
        assert_eq!(Opcode::LtI.to_byte(), 0x42);
        assert_eq!(Opcode::LeI.to_byte(), 0x43);
        assert_eq!(Opcode::GtI.to_byte(), 0x44);
        assert_eq!(Opcode::GeI.to_byte(), 0x45);

        // Control Flow (0x50-0x5F)
        assert_eq!(Opcode::Jmp.to_byte(), 0x50);
        assert_eq!(Opcode::JmpIf.to_byte(), 0x51);
        assert_eq!(Opcode::JmpNot.to_byte(), 0x52);
        assert_eq!(Opcode::Ret.to_byte(), 0x59);
        assert_eq!(Opcode::RetV.to_byte(), 0x5A);
        assert_eq!(Opcode::Call.to_byte(), 0x5B);

        // Memory (0x60-0x6F)
        assert_eq!(Opcode::New.to_byte(), 0x60);
        assert_eq!(Opcode::NewG.to_byte(), 0x61);
        assert_eq!(Opcode::GetF.to_byte(), 0x62);
        assert_eq!(Opcode::SetF.to_byte(), 0x63);

        // CBGR (0x70-0x7F)
        assert_eq!(Opcode::Ref.to_byte(), 0x70);
        assert_eq!(Opcode::RefMut.to_byte(), 0x71);
        assert_eq!(Opcode::Deref.to_byte(), 0x72);
        assert_eq!(Opcode::ChkRef.to_byte(), 0x74);

        // Tensor (0xF0-0xFF)
        assert_eq!(Opcode::TensorNew.to_byte(), 0xF0);
        assert_eq!(Opcode::TensorBinop.to_byte(), 0xF1);
        assert_eq!(Opcode::TensorMatmul.to_byte(), 0xF3);
        assert_eq!(Opcode::TensorReduce.to_byte(), 0xF4);

        // GPU (0xF8-0xFB)
        assert_eq!(Opcode::GpuExtended.to_byte(), 0xF8);
        assert_eq!(Opcode::GpuSync.to_byte(), 0xF9);
        assert_eq!(Opcode::GpuMemcpy.to_byte(), 0xFA);
        assert_eq!(Opcode::GpuAlloc.to_byte(), 0xFB);
    }

    #[test]
    fn test_opcode_from_byte() {
        assert_eq!(Opcode::from_byte(0x00), Opcode::Mov);
        assert_eq!(Opcode::from_byte(0x50), Opcode::Jmp);
        assert_eq!(Opcode::from_byte(0xF0), Opcode::TensorNew);
        assert_eq!(Opcode::from_byte(0xFC), Opcode::TensorExtended);
    }

    #[test]
    fn test_opcode_mnemonic() {
        assert_eq!(Opcode::Mov.mnemonic(), "MOV");
        assert_eq!(Opcode::LoadK.mnemonic(), "LOAD_K");
        assert_eq!(Opcode::LoadI.mnemonic(), "LOAD_I");
        assert_eq!(Opcode::AddI.mnemonic(), "ADD_I");
        assert_eq!(Opcode::SubF.mnemonic(), "SUB_F");
        assert_eq!(Opcode::Jmp.mnemonic(), "JMP");
        assert_eq!(Opcode::JmpIf.mnemonic(), "JMP_IF");
        assert_eq!(Opcode::Ret.mnemonic(), "RET");
        assert_eq!(Opcode::Call.mnemonic(), "CALL");
        assert_eq!(Opcode::TensorMatmul.mnemonic(), "TENSOR_MATMUL");
        assert_eq!(Opcode::GpuExtended.mnemonic(), "GPU_EXTENDED");
        assert_eq!(Opcode::GradBegin.mnemonic(), "GRAD_BEGIN");
        assert_eq!(Opcode::CtxGet.mnemonic(), "CTX_GET");
    }

    #[test]
    fn test_opcode_mnemonic_vllsi_ops() {
        // V-LLSI system operations (0x45-0x4F)
        assert_eq!(Opcode::SyscallLinux.mnemonic(), "SYSCALL_LINUX");
        assert_eq!(Opcode::Mmap.mnemonic(), "MMAP");
        assert_eq!(Opcode::TlsGet.mnemonic(), "TLS_GET");
        assert_eq!(Opcode::TlsSet.mnemonic(), "TLS_SET");
        // Context capability operations
        assert_eq!(Opcode::HasCapability.mnemonic(), "HAS_CAP");
    }

    #[test]
    fn test_conversion_opcodes() {
        // Conversion opcodes should have proper mnemonics
        assert_eq!(Opcode::CvtIF.mnemonic(), "CVT_IF");
        assert_eq!(Opcode::CvtFI.mnemonic(), "CVT_FI");
        assert_eq!(Opcode::CvtIC.mnemonic(), "CVT_IC");
        assert_eq!(Opcode::CvtCI.mnemonic(), "CVT_CI");
        assert_eq!(Opcode::CvtBI.mnemonic(), "CVT_BI");
    }

    #[test]
    fn test_opcode_categories() {
        // Branch instructions
        assert!(Opcode::Jmp.is_branch());
        assert!(Opcode::JmpIf.is_branch());
        assert!(Opcode::JmpNot.is_branch());
        assert!(Opcode::JmpEq.is_branch());
        assert!(Opcode::JmpNe.is_branch());
        assert!(Opcode::JmpLt.is_branch());
        assert!(Opcode::JmpLe.is_branch());
        assert!(Opcode::JmpGt.is_branch());
        assert!(Opcode::JmpGe.is_branch());
        assert!(Opcode::Switch.is_branch());
        assert!(!Opcode::Mov.is_branch());
        assert!(!Opcode::Ret.is_branch());
        assert!(!Opcode::Call.is_branch());

        // Return instructions
        assert!(Opcode::Ret.is_return());
        assert!(Opcode::RetV.is_return());
        assert!(!Opcode::Jmp.is_return());
        assert!(!Opcode::Call.is_return());

        // Call instructions
        assert!(Opcode::Call.is_call());
        assert!(Opcode::TailCall.is_call());
        assert!(Opcode::CallM.is_call());
        assert!(Opcode::CallClosure.is_call());
        assert!(Opcode::CallG.is_call());
        assert!(Opcode::CallV.is_call());
        assert!(Opcode::CallC.is_call());
        assert!(!Opcode::Ret.is_call());
        assert!(!Opcode::Jmp.is_call());

        // Tensor instructions (0xD0-0xFF)
        assert!(Opcode::TensorNew.is_tensor());
        assert!(Opcode::TensorFull.is_tensor());
        assert!(Opcode::TensorReshape.is_tensor());
        assert!(Opcode::TensorBinop.is_tensor());
        assert!(Opcode::TensorMatmul.is_tensor());
        assert!(Opcode::TensorReduce.is_tensor());
        assert!(Opcode::GpuExtended.is_tensor()); // GPU ops are in tensor range
        assert!(Opcode::TensorExtended.is_tensor());
        assert!(!Opcode::Call.is_tensor());
        assert!(!Opcode::AddI.is_tensor());

        // GPU instructions
        assert!(Opcode::GpuExtended.is_gpu());
        assert!(Opcode::GpuSync.is_gpu());
        assert!(Opcode::GpuMemcpy.is_gpu());
        assert!(Opcode::GpuAlloc.is_gpu());
        assert!(!Opcode::TensorMatmul.is_gpu());
        assert!(!Opcode::Call.is_gpu());
    }

    #[test]
    fn test_opcode_equality() {
        assert_eq!(Opcode::Mov, Opcode::Mov);
        assert_ne!(Opcode::Mov, Opcode::LoadK);
    }

    #[test]
    fn test_opcode_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Opcode::Mov);
        set.insert(Opcode::LoadK);
        set.insert(Opcode::Mov); // Duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&Opcode::Mov));
        assert!(set.contains(&Opcode::LoadK));
    }

    // ========================================================================
    // TensorDType Tests
    // ========================================================================

    #[test]
    fn test_tensor_dtype_values() {
        assert_eq!(TensorDType::F64 as u8, 0x00);
        assert_eq!(TensorDType::F32 as u8, 0x01);
        assert_eq!(TensorDType::F16 as u8, 0x02);
        assert_eq!(TensorDType::BF16 as u8, 0x03);
        assert_eq!(TensorDType::I64 as u8, 0x04);
        assert_eq!(TensorDType::I32 as u8, 0x05);
        assert_eq!(TensorDType::I16 as u8, 0x06);
        assert_eq!(TensorDType::I8 as u8, 0x07);
        assert_eq!(TensorDType::U64 as u8, 0x08);
        assert_eq!(TensorDType::U32 as u8, 0x09);
        assert_eq!(TensorDType::U16 as u8, 0x0A);
        assert_eq!(TensorDType::U8 as u8, 0x0B);
        assert_eq!(TensorDType::Bool as u8, 0x0C);
        assert_eq!(TensorDType::Complex64 as u8, 0x0D);
        assert_eq!(TensorDType::Complex128 as u8, 0x0E);
    }

    #[test]
    fn test_tensor_dtype_equality() {
        assert_eq!(TensorDType::F32, TensorDType::F32);
        assert_ne!(TensorDType::F32, TensorDType::F64);
    }

    #[test]
    fn test_tensor_dtype_clone_copy() {
        let d1 = TensorDType::F32;
        let d2 = d1; // Copy
        let d3 = d1;

        assert_eq!(d1, d2);
        assert_eq!(d1, d3);
    }

    // ========================================================================
    // TensorBinaryOp Tests
    // ========================================================================

    #[test]
    fn test_tensor_binary_op_values() {
        assert_eq!(TensorBinaryOp::Add as u8, 0x00);
        assert_eq!(TensorBinaryOp::Sub as u8, 0x01);
        assert_eq!(TensorBinaryOp::Mul as u8, 0x02);
        assert_eq!(TensorBinaryOp::Div as u8, 0x03);
        assert_eq!(TensorBinaryOp::Pow as u8, 0x04);
        assert_eq!(TensorBinaryOp::Mod as u8, 0x05);
        assert_eq!(TensorBinaryOp::Min as u8, 0x06);
        assert_eq!(TensorBinaryOp::Max as u8, 0x07);
    }

    #[test]
    fn test_tensor_binary_op_equality() {
        assert_eq!(TensorBinaryOp::Add, TensorBinaryOp::Add);
        assert_ne!(TensorBinaryOp::Add, TensorBinaryOp::Sub);
    }

    // ========================================================================
    // TensorUnaryOp Tests
    // ========================================================================

    #[test]
    fn test_tensor_unary_op_values() {
        assert_eq!(TensorUnaryOp::Neg as u8, 0x00);
        assert_eq!(TensorUnaryOp::Abs as u8, 0x01);
        assert_eq!(TensorUnaryOp::Sqrt as u8, 0x02);
        assert_eq!(TensorUnaryOp::Exp as u8, 0x03);
        assert_eq!(TensorUnaryOp::Log as u8, 0x04);
        assert_eq!(TensorUnaryOp::Sin as u8, 0x05);
        assert_eq!(TensorUnaryOp::Cos as u8, 0x06);
        assert_eq!(TensorUnaryOp::Tan as u8, 0x07);
        assert_eq!(TensorUnaryOp::Tanh as u8, 0x08);
        assert_eq!(TensorUnaryOp::Sigmoid as u8, 0x09);
        assert_eq!(TensorUnaryOp::Relu as u8, 0x0A);
        assert_eq!(TensorUnaryOp::Gelu as u8, 0x0B);
        assert_eq!(TensorUnaryOp::Silu as u8, 0x0C);
        assert_eq!(TensorUnaryOp::Floor as u8, 0x0D);
        assert_eq!(TensorUnaryOp::Ceil as u8, 0x0E);
        assert_eq!(TensorUnaryOp::Round as u8, 0x0F);
        assert_eq!(TensorUnaryOp::Sign as u8, 0x10);
        assert_eq!(TensorUnaryOp::Rsqrt as u8, 0x11);
        assert_eq!(TensorUnaryOp::Erf as u8, 0x12);
        assert_eq!(TensorUnaryOp::Log2 as u8, 0x13);
        assert_eq!(TensorUnaryOp::Softplus as u8, 0x14);
        assert_eq!(TensorUnaryOp::Mish as u8, 0x15);
    }

    #[test]
    fn test_tensor_unary_op_equality() {
        assert_eq!(TensorUnaryOp::Relu, TensorUnaryOp::Relu);
        assert_ne!(TensorUnaryOp::Relu, TensorUnaryOp::Gelu);
    }

    // ========================================================================
    // TensorReduceOp Tests
    // ========================================================================

    #[test]
    fn test_tensor_reduce_op_values() {
        assert_eq!(TensorReduceOp::Sum as u8, 0x00);
        assert_eq!(TensorReduceOp::Prod as u8, 0x01);
        assert_eq!(TensorReduceOp::Max as u8, 0x02);
        assert_eq!(TensorReduceOp::Min as u8, 0x03);
        assert_eq!(TensorReduceOp::Mean as u8, 0x04);
        assert_eq!(TensorReduceOp::Var as u8, 0x05);
        assert_eq!(TensorReduceOp::Std as u8, 0x06);
        assert_eq!(TensorReduceOp::Norm as u8, 0x07);
        assert_eq!(TensorReduceOp::LogSumExp as u8, 0x08);
        assert_eq!(TensorReduceOp::All as u8, 0x09);
        assert_eq!(TensorReduceOp::Any as u8, 0x0A);
    }

    #[test]
    fn test_tensor_reduce_op_equality() {
        assert_eq!(TensorReduceOp::Sum, TensorReduceOp::Sum);
        assert_ne!(TensorReduceOp::Sum, TensorReduceOp::Mean);
    }

    // ========================================================================
    // BinaryIntOp Tests
    // ========================================================================

    #[test]
    fn test_binary_int_op_variants() {
        let ops = [
            BinaryIntOp::Add,
            BinaryIntOp::Sub,
            BinaryIntOp::Mul,
            BinaryIntOp::Div,
            BinaryIntOp::Mod,
            BinaryIntOp::Pow,
        ];

        // Ensure all variants are distinct
        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    #[test]
    fn test_binary_int_op_clone_copy() {
        let op1 = BinaryIntOp::Add;
        let op2 = op1; // Copy
        let op3 = op1;

        assert_eq!(op1, op2);
        assert_eq!(op1, op3);
    }

    // ========================================================================
    // UnaryIntOp Tests
    // ========================================================================

    #[test]
    fn test_unary_int_op_variants() {
        let ops = [
            UnaryIntOp::Neg,
            UnaryIntOp::Abs,
            UnaryIntOp::Inc,
            UnaryIntOp::Dec,
        ];

        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    // ========================================================================
    // CompareOp Tests
    // ========================================================================

    #[test]
    fn test_compare_op_variants() {
        let ops = [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ];

        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    #[test]
    fn test_compare_op_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(CompareOp::Eq);
        set.insert(CompareOp::Lt);
        set.insert(CompareOp::Eq); // Duplicate

        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // GradMode Tests
    // ========================================================================

    #[test]
    fn test_grad_mode_variants() {
        let modes = [GradMode::Reverse, GradMode::Forward, GradMode::Auto];

        for (i, m1) in modes.iter().enumerate() {
            for (j, m2) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(m1, m2);
                } else {
                    assert_ne!(m1, m2);
                }
            }
        }
    }

    #[test]
    fn test_grad_mode_clone_copy() {
        let m1 = GradMode::Reverse;
        let m2 = m1; // Copy
        let m3 = m1;

        assert_eq!(m1, m2);
        assert_eq!(m1, m3);
    }

    // ========================================================================
    // BitwiseOp Tests
    // ========================================================================

    #[test]
    fn test_bitwise_op_variants() {
        let ops = [
            BitwiseOp::And,
            BitwiseOp::Or,
            BitwiseOp::Xor,
            BitwiseOp::Not,
            BitwiseOp::Shl,
            BitwiseOp::Shr,
            BitwiseOp::Ushr,
        ];

        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    // ========================================================================
    // BinaryFloatOp Tests
    // ========================================================================

    #[test]
    fn test_binary_float_op_variants() {
        let ops = [
            BinaryFloatOp::Add,
            BinaryFloatOp::Sub,
            BinaryFloatOp::Mul,
            BinaryFloatOp::Div,
            BinaryFloatOp::Pow,
            BinaryFloatOp::Mod,
        ];

        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    // ========================================================================
    // UnaryFloatOp Tests
    // ========================================================================

    #[test]
    fn test_unary_float_op_variants() {
        let ops = [
            UnaryFloatOp::Neg,
            UnaryFloatOp::Abs,
            UnaryFloatOp::Sqrt,
            UnaryFloatOp::Exp,
            UnaryFloatOp::Log,
            UnaryFloatOp::Sin,
            UnaryFloatOp::Cos,
            UnaryFloatOp::Tan,
            UnaryFloatOp::Floor,
            UnaryFloatOp::Ceil,
            UnaryFloatOp::Round,
        ];

        // Verify all are distinct
        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    // ========================================================================
    // BinaryGenericOp Tests
    // ========================================================================

    #[test]
    fn test_binary_generic_op_variants() {
        let ops = [
            BinaryGenericOp::Add,
            BinaryGenericOp::Sub,
            BinaryGenericOp::Mul,
            BinaryGenericOp::Div,
        ];

        for (i, op1) in ops.iter().enumerate() {
            for (j, op2) in ops.iter().enumerate() {
                if i == j {
                    assert_eq!(op1, op2);
                } else {
                    assert_ne!(op1, op2);
                }
            }
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Data Movement
    // ========================================================================

    #[test]
    fn test_instruction_mov() {
        let instr = Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        };

        if let Instruction::Mov { dst, src } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(src, Reg(1));
        } else {
            panic!("Expected Mov instruction");
        }
    }

    #[test]
    fn test_instruction_load_k() {
        let instr = Instruction::LoadK {
            dst: Reg(5),
            const_id: 42,
        };

        if let Instruction::LoadK { dst, const_id } = instr {
            assert_eq!(dst, Reg(5));
            assert_eq!(const_id, 42);
        } else {
            panic!("Expected LoadK instruction");
        }
    }

    #[test]
    fn test_instruction_load_i() {
        let instr = Instruction::LoadI {
            dst: Reg(0),
            value: -12345678901234i64,
        };

        if let Instruction::LoadI { dst, value } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(value, -12345678901234i64);
        } else {
            panic!("Expected LoadI instruction");
        }
    }

    #[test]
    fn test_instruction_load_f() {
        let instr = Instruction::LoadF {
            dst: Reg(0),
            value: 3.14159265358979,
        };

        if let Instruction::LoadF { dst, value } = instr {
            assert_eq!(dst, Reg(0));
            assert!((value - 3.14159265358979).abs() < f64::EPSILON);
        } else {
            panic!("Expected LoadF instruction");
        }
    }

    #[test]
    fn test_instruction_load_true_false() {
        let true_instr = Instruction::LoadTrue { dst: Reg(0) };
        let false_instr = Instruction::LoadFalse { dst: Reg(1) };

        assert!(matches!(true_instr, Instruction::LoadTrue { dst } if dst == Reg(0)));
        assert!(matches!(false_instr, Instruction::LoadFalse { dst } if dst == Reg(1)));
    }

    #[test]
    fn test_instruction_load_unit() {
        let instr = Instruction::LoadUnit { dst: Reg(0) };
        assert!(matches!(instr, Instruction::LoadUnit { dst } if dst == Reg(0)));
    }

    #[test]
    fn test_instruction_load_small_i() {
        let instr = Instruction::LoadSmallI {
            dst: Reg(0),
            value: -64,
        };

        if let Instruction::LoadSmallI { dst, value } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(value, -64);
        } else {
            panic!("Expected LoadSmallI instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Arithmetic
    // ========================================================================

    #[test]
    fn test_instruction_binary_i() {
        let instr = Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::BinaryI { op, dst, a, b } = instr {
            assert_eq!(op, BinaryIntOp::Add);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected BinaryI instruction");
        }
    }

    #[test]
    fn test_instruction_binary_f() {
        let instr = Instruction::BinaryF {
            op: BinaryFloatOp::Mul,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::BinaryF { op, dst, a, b } = instr {
            assert_eq!(op, BinaryFloatOp::Mul);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected BinaryF instruction");
        }
    }

    #[test]
    fn test_instruction_unary_i() {
        let instr = Instruction::UnaryI {
            op: UnaryIntOp::Neg,
            dst: Reg(0),
            src: Reg(1),
        };

        if let Instruction::UnaryI { op, dst, src } = instr {
            assert_eq!(op, UnaryIntOp::Neg);
            assert_eq!(dst, Reg(0));
            assert_eq!(src, Reg(1));
        } else {
            panic!("Expected UnaryI instruction");
        }
    }

    #[test]
    fn test_instruction_unary_f() {
        let instr = Instruction::UnaryF {
            op: UnaryFloatOp::Sqrt,
            dst: Reg(0),
            src: Reg(1),
        };

        if let Instruction::UnaryF { op, dst, src } = instr {
            assert_eq!(op, UnaryFloatOp::Sqrt);
            assert_eq!(dst, Reg(0));
            assert_eq!(src, Reg(1));
        } else {
            panic!("Expected UnaryF instruction");
        }
    }

    #[test]
    fn test_instruction_not() {
        let instr = Instruction::Not {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(matches!(instr, Instruction::Not { dst, src } if dst == Reg(0) && src == Reg(1)));
    }

    #[test]
    fn test_instruction_bitwise() {
        let instr = Instruction::Bitwise {
            op: BitwiseOp::Xor,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::Bitwise { op, dst, a, b } = instr {
            assert_eq!(op, BitwiseOp::Xor);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected Bitwise instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Comparison
    // ========================================================================

    #[test]
    fn test_instruction_cmp_i() {
        let instr = Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::CmpI { op, dst, a, b } = instr {
            assert_eq!(op, CompareOp::Lt);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected CmpI instruction");
        }
    }

    #[test]
    fn test_instruction_cmp_f() {
        let instr = Instruction::CmpF {
            op: CompareOp::Ge,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::CmpF { op, dst, a, b } = instr {
            assert_eq!(op, CompareOp::Ge);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected CmpF instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Control Flow
    // ========================================================================

    #[test]
    fn test_instruction_jmp() {
        let instr = Instruction::Jmp { offset: -10 };
        assert!(matches!(instr, Instruction::Jmp { offset } if offset == -10));
    }

    #[test]
    fn test_instruction_jmp_if() {
        let instr = Instruction::JmpIf {
            cond: Reg(0),
            offset: 5,
        };

        if let Instruction::JmpIf { cond, offset } = instr {
            assert_eq!(cond, Reg(0));
            assert_eq!(offset, 5);
        } else {
            panic!("Expected JmpIf instruction");
        }
    }

    #[test]
    fn test_instruction_jmp_not() {
        let instr = Instruction::JmpNot {
            cond: Reg(0),
            offset: -3,
        };

        if let Instruction::JmpNot { cond, offset } = instr {
            assert_eq!(cond, Reg(0));
            assert_eq!(offset, -3);
        } else {
            panic!("Expected JmpNot instruction");
        }
    }

    #[test]
    fn test_instruction_jmp_cmp() {
        let instr = Instruction::JmpCmp {
            op: CompareOp::Eq,
            a: Reg(0),
            b: Reg(1),
            offset: 10,
        };

        if let Instruction::JmpCmp { op, a, b, offset } = instr {
            assert_eq!(op, CompareOp::Eq);
            assert_eq!(a, Reg(0));
            assert_eq!(b, Reg(1));
            assert_eq!(offset, 10);
        } else {
            panic!("Expected JmpCmp instruction");
        }
    }

    #[test]
    fn test_instruction_ret() {
        let instr = Instruction::Ret { value: Reg(0) };
        assert!(matches!(instr, Instruction::Ret { value } if value == Reg(0)));
    }

    #[test]
    fn test_instruction_ret_v() {
        let instr = Instruction::RetV;
        assert!(matches!(instr, Instruction::RetV));
    }

    #[test]
    fn test_instruction_call() {
        let instr = Instruction::Call {
            dst: Reg(0),
            func_id: 123,
            args: RegRange::new(Reg(1), 3),
        };

        if let Instruction::Call { dst, func_id, args } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(func_id, 123);
            assert_eq!(args.start, Reg(1));
            assert_eq!(args.count, 3);
        } else {
            panic!("Expected Call instruction");
        }
    }

    #[test]
    fn test_instruction_tail_call() {
        let instr = Instruction::TailCall {
            func_id: 456,
            args: RegRange::new(Reg(0), 2),
        };

        if let Instruction::TailCall { func_id, args } = instr {
            assert_eq!(func_id, 456);
            assert_eq!(args.start, Reg(0));
            assert_eq!(args.count, 2);
        } else {
            panic!("Expected TailCall instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Memory
    // ========================================================================

    #[test]
    fn test_instruction_new() {
        let instr = Instruction::New {
            dst: Reg(0),
            type_id: 42,
            field_count: 3,
        };

        if let Instruction::New { dst, type_id, field_count } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(type_id, 42);
            assert_eq!(field_count, 3);
        } else {
            panic!("Expected New instruction");
        }
    }

    #[test]
    fn test_instruction_get_f() {
        let instr = Instruction::GetF {
            dst: Reg(0),
            obj: Reg(1),
            field_idx: 3,
        };

        if let Instruction::GetF {
            dst,
            obj,
            field_idx,
        } = instr
        {
            assert_eq!(dst, Reg(0));
            assert_eq!(obj, Reg(1));
            assert_eq!(field_idx, 3);
        } else {
            panic!("Expected GetF instruction");
        }
    }

    #[test]
    fn test_instruction_set_f() {
        let instr = Instruction::SetF {
            obj: Reg(0),
            field_idx: 2,
            value: Reg(1),
        };

        if let Instruction::SetF {
            obj,
            field_idx,
            value,
        } = instr
        {
            assert_eq!(obj, Reg(0));
            assert_eq!(field_idx, 2);
            assert_eq!(value, Reg(1));
        } else {
            panic!("Expected SetF instruction");
        }
    }

    #[test]
    fn test_instruction_get_e() {
        let instr = Instruction::GetE {
            dst: Reg(0),
            arr: Reg(1),
            idx: Reg(2),
        };

        if let Instruction::GetE { dst, arr, idx } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(arr, Reg(1));
            assert_eq!(idx, Reg(2));
        } else {
            panic!("Expected GetE instruction");
        }
    }

    #[test]
    fn test_instruction_set_e() {
        let instr = Instruction::SetE {
            arr: Reg(0),
            idx: Reg(1),
            value: Reg(2),
        };

        if let Instruction::SetE { arr, idx, value } = instr {
            assert_eq!(arr, Reg(0));
            assert_eq!(idx, Reg(1));
            assert_eq!(value, Reg(2));
        } else {
            panic!("Expected SetE instruction");
        }
    }

    #[test]
    fn test_instruction_len() {
        let instr = Instruction::Len {
            dst: Reg(0),
            arr: Reg(1),
            type_hint: 0,
        };

        if let Instruction::Len { dst, arr, type_hint } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(arr, Reg(1));
            assert_eq!(type_hint, 0);
        } else {
            panic!("Expected Len instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - CBGR
    // ========================================================================

    #[test]
    fn test_instruction_ref() {
        let instr = Instruction::Ref {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(matches!(instr, Instruction::Ref { dst, src } if dst == Reg(0) && src == Reg(1)));
    }

    #[test]
    fn test_instruction_ref_mut() {
        let instr = Instruction::RefMut {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::RefMut { dst, src } if dst == Reg(0) && src == Reg(1))
        );
    }

    #[test]
    fn test_instruction_deref() {
        let instr = Instruction::Deref {
            dst: Reg(0),
            ref_reg: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::Deref { dst, ref_reg } if dst == Reg(0) && ref_reg == Reg(1))
        );
    }

    #[test]
    fn test_instruction_chk_ref() {
        let instr = Instruction::ChkRef { ref_reg: Reg(0) };
        assert!(matches!(instr, Instruction::ChkRef { ref_reg } if ref_reg == Reg(0)));
    }

    #[test]
    fn test_instruction_ref_checked() {
        let instr = Instruction::RefChecked {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::RefChecked { dst, src } if dst == Reg(0) && src == Reg(1))
        );
    }

    #[test]
    fn test_instruction_ref_unsafe() {
        let instr = Instruction::RefUnsafe {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::RefUnsafe { dst, src } if dst == Reg(0) && src == Reg(1))
        );
    }

    // ========================================================================
    // Instruction Variant Tests - Pattern Matching
    // ========================================================================

    #[test]
    fn test_instruction_is_var() {
        let instr = Instruction::IsVar {
            dst: Reg(0),
            value: Reg(1),
            tag: 5,
        };

        if let Instruction::IsVar { dst, value, tag } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(value, Reg(1));
            assert_eq!(tag, 5);
        } else {
            panic!("Expected IsVar instruction");
        }
    }

    #[test]
    fn test_instruction_as_var() {
        let instr = Instruction::AsVar {
            dst: Reg(0),
            value: Reg(1),
            tag: 3,
        };

        if let Instruction::AsVar { dst, value, tag } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(value, Reg(1));
            assert_eq!(tag, 3);
        } else {
            panic!("Expected AsVar instruction");
        }
    }

    #[test]
    fn test_instruction_unpack() {
        let instr = Instruction::Unpack {
            dst_start: Reg(0),
            tuple: Reg(5),
            count: 3,
        };

        if let Instruction::Unpack {
            dst_start,
            tuple,
            count,
        } = instr
        {
            assert_eq!(dst_start, Reg(0));
            assert_eq!(tuple, Reg(5));
            assert_eq!(count, 3);
        } else {
            panic!("Expected Unpack instruction");
        }
    }

    #[test]
    fn test_instruction_pack() {
        let instr = Instruction::Pack {
            dst: Reg(5),
            src_start: Reg(0),
            count: 3,
        };

        if let Instruction::Pack {
            dst,
            src_start,
            count,
        } = instr
        {
            assert_eq!(dst, Reg(5));
            assert_eq!(src_start, Reg(0));
            assert_eq!(count, 3);
        } else {
            panic!("Expected Pack instruction");
        }
    }

    #[test]
    fn test_instruction_switch() {
        let instr = Instruction::Switch {
            value: Reg(0),
            default_offset: 100,
            cases: vec![(0, 10), (1, 20), (2, 30)],
        };

        if let Instruction::Switch {
            value,
            default_offset,
            cases,
        } = instr
        {
            assert_eq!(value, Reg(0));
            assert_eq!(default_offset, 100);
            assert_eq!(cases.len(), 3);
            assert_eq!(cases[0], (0, 10));
            assert_eq!(cases[1], (1, 20));
            assert_eq!(cases[2], (2, 30));
        } else {
            panic!("Expected Switch instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Async
    // ========================================================================

    #[test]
    fn test_instruction_spawn() {
        let instr = Instruction::Spawn {
            dst: Reg(0),
            func_id: 42,
            args: RegRange::new(Reg(1), 2),
        };

        if let Instruction::Spawn { dst, func_id, args } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(func_id, 42);
            assert_eq!(args.start, Reg(1));
            assert_eq!(args.count, 2);
        } else {
            panic!("Expected Spawn instruction");
        }
    }

    #[test]
    fn test_instruction_await() {
        let instr = Instruction::Await {
            dst: Reg(0),
            task: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::Await { dst, task } if dst == Reg(0) && task == Reg(1))
        );
    }

    #[test]
    fn test_instruction_yield() {
        let instr = Instruction::Yield { value: Reg(0) };
        assert!(matches!(instr, Instruction::Yield { value } if value == Reg(0)));
    }

    #[test]
    fn test_instruction_select() {
        let instr = Instruction::Select {
            dst: Reg(0),
            futures: vec![Reg(1), Reg(2), Reg(3)],
            handlers: vec![10, 20, 30],
        };

        if let Instruction::Select {
            dst,
            futures,
            handlers,
        } = instr
        {
            assert_eq!(dst, Reg(0));
            assert_eq!(futures, vec![Reg(1), Reg(2), Reg(3)]);
            assert_eq!(handlers, vec![10, 20, 30]);
        } else {
            panic!("Expected Select instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Autodiff
    // ========================================================================

    #[test]
    fn test_instruction_grad_begin() {
        let instr = Instruction::GradBegin {
            scope_id: 1,
            mode: GradMode::Reverse,
            wrt: vec![Reg(0), Reg(1)],
        };

        if let Instruction::GradBegin {
            scope_id,
            mode,
            wrt,
        } = instr
        {
            assert_eq!(scope_id, 1);
            assert_eq!(mode, GradMode::Reverse);
            assert_eq!(wrt, vec![Reg(0), Reg(1)]);
        } else {
            panic!("Expected GradBegin instruction");
        }
    }

    #[test]
    fn test_instruction_grad_end() {
        let instr = Instruction::GradEnd {
            scope_id: 1,
            output: Reg(0),
            grad_out: Reg(1),
            grad_regs: vec![Reg(2), Reg(3)],
        };

        if let Instruction::GradEnd {
            scope_id,
            output,
            grad_out,
            grad_regs,
        } = instr
        {
            assert_eq!(scope_id, 1);
            assert_eq!(output, Reg(0));
            assert_eq!(grad_out, Reg(1));
            assert_eq!(grad_regs, vec![Reg(2), Reg(3)]);
        } else {
            panic!("Expected GradEnd instruction");
        }
    }

    #[test]
    fn test_instruction_grad_checkpoint() {
        let instr = Instruction::GradCheckpoint {
            id: 42,
            tensors: vec![Reg(0), Reg(1), Reg(2)],
        };

        if let Instruction::GradCheckpoint { id, tensors } = instr {
            assert_eq!(id, 42);
            assert_eq!(tensors, vec![Reg(0), Reg(1), Reg(2)]);
        } else {
            panic!("Expected GradCheckpoint instruction");
        }
    }

    #[test]
    fn test_instruction_grad_accumulate() {
        let instr = Instruction::GradAccumulate {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::GradAccumulate { dst, src } if dst == Reg(0) && src == Reg(1))
        );
    }

    #[test]
    fn test_instruction_grad_stop() {
        let instr = Instruction::GradStop {
            dst: Reg(0),
            src: Reg(1),
        };

        assert!(
            matches!(instr, Instruction::GradStop { dst, src } if dst == Reg(0) && src == Reg(1))
        );
    }

    // ========================================================================
    // Instruction Variant Tests - Context
    // ========================================================================

    #[test]
    fn test_instruction_ctx_get() {
        let instr = Instruction::CtxGet {
            dst: Reg(0),
            ctx_type: 5,
        };

        if let Instruction::CtxGet { dst, ctx_type } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(ctx_type, 5);
        } else {
            panic!("Expected CtxGet instruction");
        }
    }

    #[test]
    fn test_instruction_ctx_provide() {
        let instr = Instruction::CtxProvide {
            ctx_type: 5,
            value: Reg(0),
            body_offset: 20,
        };

        if let Instruction::CtxProvide {
            ctx_type,
            value,
            body_offset,
        } = instr
        {
            assert_eq!(ctx_type, 5);
            assert_eq!(value, Reg(0));
            assert_eq!(body_offset, 20);
        } else {
            panic!("Expected CtxProvide instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Debug/Verify
    // ========================================================================

    #[test]
    fn test_instruction_assert() {
        let instr = Instruction::Assert {
            cond: Reg(0),
            message_id: 42,
        };

        if let Instruction::Assert { cond, message_id } = instr {
            assert_eq!(cond, Reg(0));
            assert_eq!(message_id, 42);
        } else {
            panic!("Expected Assert instruction");
        }
    }

    #[test]
    fn test_instruction_panic() {
        let instr = Instruction::Panic { message_id: 99 };
        assert!(matches!(instr, Instruction::Panic { message_id } if message_id == 99));
    }

    #[test]
    fn test_instruction_unreachable() {
        let instr = Instruction::Unreachable;
        assert!(matches!(instr, Instruction::Unreachable));
    }

    #[test]
    fn test_instruction_spec() {
        let instr = Instruction::Spec {
            reg: Reg(0),
            expected_type: 10,
        };

        if let Instruction::Spec { reg, expected_type } = instr {
            assert_eq!(reg, Reg(0));
            assert_eq!(expected_type, 10);
        } else {
            panic!("Expected Spec instruction");
        }
    }

    #[test]
    fn test_instruction_guard() {
        let instr = Instruction::Guard {
            reg: Reg(0),
            expected_type: 10,
            deopt_offset: 50,
        };

        if let Instruction::Guard {
            reg,
            expected_type,
            deopt_offset,
        } = instr
        {
            assert_eq!(reg, Reg(0));
            assert_eq!(expected_type, 10);
            assert_eq!(deopt_offset, 50);
        } else {
            panic!("Expected Guard instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - Tensor Operations
    // ========================================================================

    #[test]
    fn test_instruction_tensor_new() {
        let instr = Instruction::TensorNew {
            dst: Reg(0),
            dtype: TensorDType::F32,
            dims: vec![Reg(1), Reg(2), Reg(3)],
        };

        if let Instruction::TensorNew { dst, dtype, dims } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(dtype, TensorDType::F32);
            assert_eq!(dims, vec![Reg(1), Reg(2), Reg(3)]);
        } else {
            panic!("Expected TensorNew instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_binop() {
        let instr = Instruction::TensorBinop {
            op: TensorBinaryOp::Add,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::TensorBinop { op, dst, a, b } = instr {
            assert_eq!(op, TensorBinaryOp::Add);
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected TensorBinop instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_unop() {
        let instr = Instruction::TensorUnop {
            op: TensorUnaryOp::Relu,
            dst: Reg(0),
            src: Reg(1),
        };

        if let Instruction::TensorUnop { op, dst, src } = instr {
            assert_eq!(op, TensorUnaryOp::Relu);
            assert_eq!(dst, Reg(0));
            assert_eq!(src, Reg(1));
        } else {
            panic!("Expected TensorUnop instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_matmul() {
        let instr = Instruction::TensorMatmul {
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        };

        if let Instruction::TensorMatmul { dst, a, b } = instr {
            assert_eq!(dst, Reg(0));
            assert_eq!(a, Reg(1));
            assert_eq!(b, Reg(2));
        } else {
            panic!("Expected TensorMatmul instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_reduce() {
        let instr = Instruction::TensorReduce {
            op: TensorReduceOp::Sum,
            dst: Reg(0),
            src: Reg(1),
            axes: vec![0, 1],
            keepdim: true,
        };

        if let Instruction::TensorReduce {
            op,
            dst,
            src,
            axes,
            keepdim,
        } = instr
        {
            assert_eq!(op, TensorReduceOp::Sum);
            assert_eq!(dst, Reg(0));
            assert_eq!(src, Reg(1));
            assert_eq!(axes, vec![0, 1]);
            assert!(keepdim);
        } else {
            panic!("Expected TensorReduce instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_flash_attention() {
        let instr = Instruction::TensorFlashAttention {
            dst: Reg(0),
            q: Reg(1),
            k: Reg(2),
            v: Reg(3),
            mask: Some(Reg(4)),
            scale: Reg(5),
            causal: true,
        };

        if let Instruction::TensorFlashAttention {
            dst,
            q,
            k,
            v,
            mask,
            scale,
            causal,
        } = instr
        {
            assert_eq!(dst, Reg(0));
            assert_eq!(q, Reg(1));
            assert_eq!(k, Reg(2));
            assert_eq!(v, Reg(3));
            assert_eq!(mask, Some(Reg(4)));
            assert_eq!(scale, Reg(5));
            assert!(causal);
        } else {
            panic!("Expected TensorFlashAttention instruction");
        }
    }

    #[test]
    fn test_instruction_tensor_flash_attention_no_mask() {
        let instr = Instruction::TensorFlashAttention {
            dst: Reg(0),
            q: Reg(1),
            k: Reg(2),
            v: Reg(3),
            mask: None,
            scale: Reg(4),
            causal: false,
        };

        if let Instruction::TensorFlashAttention { mask, causal, .. } = instr {
            assert_eq!(mask, None);
            assert!(!causal);
        } else {
            panic!("Expected TensorFlashAttention instruction");
        }
    }

    // ========================================================================
    // Instruction Variant Tests - GPU
    // ========================================================================

    #[test]
    fn test_instruction_gpu_launch() {
        let instr = Instruction::GpuLaunch {
            kernel_id: 42,
            grid: [Reg(0), Reg(1), Reg(2)],
            block: [Reg(3), Reg(4), Reg(5)],
            shared_mem: Reg(6),
            stream: Reg(7),
            args: vec![Reg(8), Reg(9), Reg(10)],
        };

        if let Instruction::GpuLaunch {
            kernel_id,
            grid,
            block,
            shared_mem,
            stream,
            args,
        } = instr
        {
            assert_eq!(kernel_id, 42);
            assert_eq!(grid, [Reg(0), Reg(1), Reg(2)]);
            assert_eq!(block, [Reg(3), Reg(4), Reg(5)]);
            assert_eq!(shared_mem, Reg(6));
            assert_eq!(stream, Reg(7));
            assert_eq!(args, vec![Reg(8), Reg(9), Reg(10)]);
        } else {
            panic!("Expected GpuLaunch instruction");
        }
    }

    #[test]
    fn test_instruction_gpu_sync() {
        let instr = Instruction::GpuSync { stream: Reg(0) };
        assert!(matches!(instr, Instruction::GpuSync { stream } if stream == Reg(0)));
    }

    // ========================================================================
    // Instruction Variant Tests - Raw
    // ========================================================================

    #[test]
    fn test_instruction_raw() {
        let instr = Instruction::Raw {
            opcode: Opcode::Nop,
            data: vec![0x01, 0x02, 0x03],
        };

        if let Instruction::Raw { opcode, data } = instr {
            assert_eq!(opcode, Opcode::Nop);
            assert_eq!(data, vec![0x01, 0x02, 0x03]);
        } else {
            panic!("Expected Raw instruction");
        }
    }

    // ========================================================================
    // Instruction Equality and Clone Tests
    // ========================================================================

    #[test]
    fn test_instruction_equality() {
        let i1 = Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        };
        let i2 = Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        };
        let i3 = Instruction::Mov {
            dst: Reg(0),
            src: Reg(2),
        };

        assert_eq!(i1, i2);
        assert_ne!(i1, i3);
    }

    #[test]
    fn test_instruction_clone() {
        let original = Instruction::Call {
            dst: Reg(0),
            func_id: 42,
            args: RegRange::new(Reg(1), 3),
        };

        let cloned = original.clone();
        assert_eq!(original, cloned);
    }
}
