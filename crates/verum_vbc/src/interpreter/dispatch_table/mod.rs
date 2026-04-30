//! Function table dispatch for VBC interpreter.
//!
//! This module implements an optimized dispatch mechanism using a pre-computed
//! function pointer table. Instead of a large match statement, we use direct
//! array indexing for O(1) dispatch.
//!
//! # Performance
//!
//! - Switch-based dispatch: ~5-8 cycles per instruction (branch prediction misses)
//! - Function table dispatch: ~2-3 cycles per instruction (indirect call, predictable)
//!
//! # Architecture
//!
//! ```text
//! opcode byte → DISPATCH_TABLE[opcode] → handler function → DispatchResult
//! ```
//!
//! Interpreter dispatch optimization: pre-computed function pointer table indexed by opcode
//! byte (0x00-0xFF). Each handler reads operands from the bytecode stream, executes the
//! operation, and returns a DispatchResult. Expected throughput improvement of 30-50% over
//! match-based dispatch. Sub-opcodes (for extended instructions like ArithExtended 0xBD,
//! TensorExtended 0xFE, etc.) use a secondary dispatch within the handler.

use crate::instruction::Reg;
use crate::module::{ConstId, Constant, FunctionId};
use crate::types::TypeId;
use crate::value::Value;

use super::error::{InterpreterError, InterpreterResult};
use super::state::{InterpreterState, TaskId};

mod handlers;

// ============================================================================
// Extracted Handler Imports
// ============================================================================

use handlers::data_movement::{
    handle_mov, handle_loadk, handle_loadi, handle_loadf,
    handle_load_true, handle_load_false, handle_load_unit,
    handle_loadt, handle_load_smalli, handle_load_nil, handle_nop,
    handle_cvt_if, handle_cvt_fi, handle_cvt_ic, handle_cvt_ci,
    handle_cvt_bi, handle_cvt_toi, handle_cvt_tof,
};
use handlers::integer_arith::{
    handle_addi, handle_subi, handle_muli, handle_divi, handle_modi,
    handle_negi, handle_powi, handle_absi, handle_inc, handle_dec,
    handle_udivi, handle_umodi,
};
use handlers::float_arith::{
    handle_addf, handle_subf, handle_mulf, handle_divf,
    handle_negf, handle_powf, handle_modf, handle_absf,
};
use handlers::bitwise::{
    handle_addg, handle_subg, handle_mulg, handle_divg,
    handle_band, handle_bor, handle_bxor, handle_shl, handle_shr, handle_ushr, handle_bnot,
};
use handlers::comparison::{
    handle_eqi, handle_nei, handle_lti, handle_lei, handle_gti, handle_gei,
    handle_eqf, handle_nef, handle_ltf, handle_lef, handle_gtf, handle_gef,
    handle_eqg, handle_cmpg, handle_eqref, handle_cmp_extended,
};
use handlers::control_flow::{
    handle_land, handle_lor, handle_lnot, handle_lxor,
    handle_jump, handle_jump_if, handle_jump_if_not,
    handle_jump_eq, handle_jump_ne, handle_jump_lt, handle_jump_le,
    handle_jump_gt, handle_jump_ge, handle_return, handle_return_unit,
};
use handlers::debug::{
    handle_assert, handle_panic, handle_unreachable, handle_debug_print,
    handle_spec, handle_guard, handle_requires, handle_ensures, handle_invariant,
};

// Re-export format_value_for_print so it's accessible to handler modules via super::super::
pub(crate) use handlers::debug::format_value_for_print;

// Re-export value_hash and value_eq from memory_collections for method_dispatch
pub(crate) use handlers::memory_collections::{value_hash, value_eq};

use handlers::memory_collections::{
    handle_new, handle_new_array, handle_get_field, handle_set_field,
    handle_get_index, handle_set_index, handle_array_len,
    handle_new_generic, handle_new_list, handle_list_push, handle_list_pop,
    handle_new_map, handle_map_get, handle_map_set, handle_map_contains,
    handle_clone, handle_new_set, handle_set_insert, handle_set_contains,
    handle_set_remove, handle_new_deque, handle_new_channel,
    handle_push, handle_pop,
};
use handlers::cbgr::{
    handle_ref_create, handle_ref_mut, handle_deref, handle_deref_mut,
    handle_chk_ref, handle_ref_checked, handle_ref_unsafe, handle_drop_ref,
    handle_cbgr_extended,
};
use handlers::pattern_matching::{
    handle_as_var, handle_switch, handle_match_guard,
    handle_specialize, handle_type_of, handle_size_of, handle_align_of,
    handle_make_variant, handle_set_variant_data, handle_get_variant_data,
    handle_get_variant_data_ref, handle_match_tag, handle_get_tag,
    handle_unpack, handle_pack,
    handle_make_pi, handle_make_sigma, handle_make_witness,
};
use handlers::iterators::{
    handle_iter_new, handle_iter_next, handle_new_range,
};
use handlers::string_ops::{
    handle_to_string, handle_concat, handle_char_to_str,
};
use handlers::generators::{
    handle_generator_create, handle_generator_yield,
    handle_generator_next, handle_generator_has_next,
};
use handlers::exceptions::{
    handle_throw, handle_try_begin, handle_try_end, handle_get_exception,
};

// Call operations
use handlers::calls::{
    handle_call, handle_call_indirect, handle_call_generic,
    handle_call_virtual, handle_call_cached, handle_call_closure,
    handle_tail_call_op, handle_new_closure,
};

// Method dispatch
use handlers::method_dispatch::handle_call_method;

// Async + Nursery operations
use handlers::async_nursery::{
    handle_spawn, handle_await, handle_select, handle_join,
    handle_future_ready, handle_future_get, handle_async_next,
    handle_nursery_init, handle_nursery_spawn, handle_nursery_await,
    handle_nursery_cancel, handle_nursery_config, handle_nursery_error,
};

// Context + Capability operations
use handlers::context::{
    handle_ctx_get, handle_ctx_provide, handle_ctx_pop, handle_ctx_push,
    handle_attenuate, handle_has_capability, handle_require_capability,
};

// Meta operations
use handlers::meta::{
    handle_meta_eval, handle_meta_quote, handle_meta_splice, handle_meta_reflect,
};

// System operations
use handlers::system::{
    handle_syscall, handle_atomic_load, handle_atomic_store,
    handle_atomic_cas, handle_atomic_fence,
    handle_tls_get, handle_tls_set,
    handle_grad_begin, handle_grad_end, handle_grad_checkpoint,
    handle_grad_accumulate, handle_grad_stop,
    handle_mmap, handle_munmap,
    handle_io_submit, handle_io_poll,
};

// Extended opcode handlers
use handlers::text_extended::handle_text_extended;
use handlers::ffi_extended::handle_ffi_extended;
use handlers::math_extended::handle_math_extended;
use handlers::simd_extended::handle_simd_extended;
use handlers::char_extended::handle_char_extended;
use handlers::log_extended::{handle_log_extended, handle_mem_extended};
use handlers::arith_extended::handle_arith_extended;

// Tensor operations
use handlers::tensor::{
    handle_tensor_new, handle_tensor_binop, handle_tensor_unop,
    handle_tensor_matmul, handle_tensor_reduce, handle_tensor_reshape,
    handle_tensor_transpose, handle_tensor_slice,
    handle_tensor_full, handle_tensor_from_slice,
};
use handlers::gpu::{
    handle_gpu_extended, handle_gpu_sync, handle_gpu_memcpy, handle_gpu_alloc,
};
use handlers::cubical::handle_cubical_extended;
use handlers::tensor_extended::handle_tensor_extended;
use handlers::ml_extended::handle_ml_extended;
use handlers::extended::handle_extended;

// ============================================================================
// Dispatch Result
// ============================================================================

/// Result of executing a single instruction handler.
///
/// This enum allows handlers to communicate control flow decisions back to
/// the dispatch loop without using exceptions or early returns.
#[derive(Debug)]
pub enum DispatchResult {
    /// Continue to next instruction (most common case).
    Continue,
    /// Return from current function with a value.
    /// The dispatch loop should handle the return and potentially continue.
    Return(Value),
    /// Final return - exit the dispatch loop with this value.
    FinalReturn(Value),
    /// Yield from generator (suspend execution).
    Yield(Value),
}

/// Handler function type for opcode dispatch.
///
/// Each handler reads its operands from bytecode (via state), executes the
/// operation, and returns a DispatchResult indicating what the loop should do.
pub type Handler = fn(&mut InterpreterState) -> InterpreterResult<DispatchResult>;

// ============================================================================
// Dispatch Table
// ============================================================================

/// Static dispatch table mapping opcode bytes to handler functions.
///
/// This is a 256-entry array for O(1) lookup. Unimplemented opcodes map to
/// `handle_not_implemented` which returns an appropriate error.
pub static DISPATCH_TABLE: [Handler; 256] = build_dispatch_table();

/// Builds the dispatch table at compile time.
///
/// CRITICAL: Opcode mappings MUST match instruction.rs definitions exactly.
/// See instruction.rs for the authoritative opcode layout.
const fn build_dispatch_table() -> [Handler; 256] {
    let mut table: [Handler; 256] = [handle_not_implemented; 256];

    // ========================================================================
    // Data Movement (0x00-0x0F) - matches instruction.rs
    // ========================================================================
    table[0x00] = handle_mov;         // Mov = 0x00
    table[0x01] = handle_loadk;       // LoadK = 0x01
    table[0x02] = handle_loadi;       // LoadI = 0x02
    table[0x03] = handle_loadf;       // LoadF = 0x03
    table[0x04] = handle_load_true;   // LoadTrue = 0x04
    table[0x05] = handle_load_false;  // LoadFalse = 0x05
    table[0x06] = handle_load_unit;   // LoadUnit = 0x06
    table[0x07] = handle_loadt;       // LoadT = 0x07
    table[0x08] = handle_load_smalli; // LoadSmallI = 0x08
    table[0x09] = handle_load_nil;    // LoadNil = 0x09
    table[0x0A] = handle_nop;         // Nop = 0x0A
    table[0x0B] = handle_cvt_if;      // CvtIF = 0x0B
    table[0x0C] = handle_cvt_fi;      // CvtFI = 0x0C
    table[0x0D] = handle_cvt_ic;      // CvtIC = 0x0D
    table[0x0E] = handle_cvt_ci;      // CvtCI = 0x0E
    table[0x0F] = handle_cvt_bi;      // CvtBI = 0x0F

    // ========================================================================
    // Integer Arithmetic (0x10-0x1F) - matches instruction.rs
    // ========================================================================
    table[0x10] = handle_addi;        // AddI = 0x10
    table[0x11] = handle_subi;        // SubI = 0x11
    table[0x12] = handle_muli;        // MulI = 0x12
    table[0x13] = handle_divi;        // DivI = 0x13
    table[0x14] = handle_modi;        // ModI = 0x14
    table[0x15] = handle_negi;        // NegI = 0x15
    table[0x16] = handle_absi;        // AbsI = 0x16
    table[0x17] = handle_powi;        // PowI = 0x17
    table[0x18] = handle_inc;         // Inc = 0x18
    table[0x19] = handle_dec;         // Dec = 0x19
    table[0x1A] = handle_cvt_toi;     // CvtToI = 0x1A (dynamic int conversion)
    table[0x1B] = handle_udivi;       // UDivI = 0x1B (unsigned i64-as-u64 division)
    table[0x1C] = handle_umodi;       // UModI = 0x1C (unsigned i64-as-u64 modulo)
    // 0x1D-0x1E: Reserved integer arithmetic
    table[0x1F] = handle_extended;    // Extended = 0x1F (#167 Part A)

    // ========================================================================
    // Float Arithmetic (0x20-0x2F) - matches instruction.rs
    // ========================================================================
    table[0x20] = handle_addf;        // AddF = 0x20
    table[0x21] = handle_subf;        // SubF = 0x21
    table[0x22] = handle_mulf;        // MulF = 0x22
    table[0x23] = handle_divf;        // DivF = 0x23
    table[0x24] = handle_modf;        // ModF = 0x24
    table[0x25] = handle_negf;        // NegF = 0x25
    table[0x26] = handle_absf;        // AbsF = 0x26
    table[0x27] = handle_powf;        // PowF = 0x27
    table[0x28] = handle_cvt_tof;     // CvtToF = 0x28 (dynamic float conversion)

    // Math Extended (0x29) - Transcendental and special math functions
    table[0x29] = handle_math_extended;
    // SIMD Extended (0x2A) - Platform-agnostic SIMD operations
    table[0x2A] = handle_simd_extended;
    // Char Extended (0x2B) - Character classification and conversion
    table[0x2B] = handle_char_extended;
    // 0x2C-0x2F: Reserved float arithmetic

    // ========================================================================
    // Bitwise + Generic Arithmetic (0x30-0x3F) - matches instruction.rs
    // ========================================================================
    table[0x30] = handle_band;        // Band = 0x30
    table[0x31] = handle_bor;         // Bor = 0x31
    table[0x32] = handle_bxor;        // Bxor = 0x32
    table[0x33] = handle_bnot;        // Bnot = 0x33
    table[0x34] = handle_shl;         // Shl = 0x34
    table[0x35] = handle_shr;         // Shr = 0x35 (arithmetic)
    table[0x36] = handle_ushr;        // Ushr = 0x36 (logical)
    // 0x37: Reserved bitwise
    table[0x38] = handle_addg;        // AddG = 0x38 (generic add via Add protocol)
    table[0x39] = handle_subg;        // SubG = 0x39 (generic sub via Sub protocol)
    table[0x3A] = handle_mulg;        // MulG = 0x3A (generic mul via Mul protocol)
    table[0x3B] = handle_divg;        // DivG = 0x3B (generic div via Div protocol)
    // 0x3C-0x3F: Reserved generic arithmetic

    // ========================================================================
    // Comparison (0x40-0x4F) - matches instruction.rs
    // ========================================================================
    table[0x40] = handle_eqi;         // EqI = 0x40
    table[0x41] = handle_nei;         // NeI = 0x41
    table[0x42] = handle_lti;         // LtI = 0x42
    table[0x43] = handle_lei;         // LeI = 0x43
    table[0x44] = handle_gti;         // GtI = 0x44
    table[0x45] = handle_gei;         // GeI = 0x45
    table[0x46] = handle_eqf;         // EqF = 0x46
    table[0x47] = handle_nef;         // NeF = 0x47
    table[0x48] = handle_ltf;         // LtF = 0x48
    table[0x49] = handle_lef;         // LeF = 0x49
    table[0x4A] = handle_gtf;         // GtF = 0x4A
    table[0x4B] = handle_gef;         // GeF = 0x4B
    table[0x4C] = handle_eqg;         // EqG = 0x4C (generic equality via Eq protocol)
    table[0x4D] = handle_cmpg;        // CmpG = 0x4D (generic compare via Ord protocol)
    table[0x4E] = handle_eqref;       // EqRef = 0x4E (reference equality)
    table[0x4F] = handle_cmp_extended; // CmpExtended = 0x4F (unsigned comparisons)

    // ========================================================================
    // Control Flow (0x50-0x5F) - matches instruction.rs
    // ========================================================================
    table[0x50] = handle_jump;            // Jmp = 0x50
    table[0x51] = handle_jump_if;         // JmpIf = 0x51
    table[0x52] = handle_jump_if_not;     // JmpNot = 0x52
    table[0x53] = handle_jump_eq;         // JmpEq = 0x53 (fused compare-jump)
    table[0x54] = handle_jump_ne;         // JmpNe = 0x54
    table[0x55] = handle_jump_lt;         // JmpLt = 0x55
    table[0x56] = handle_jump_le;         // JmpLe = 0x56
    table[0x57] = handle_jump_gt;         // JmpGt = 0x57
    table[0x58] = handle_jump_ge;         // JmpGe = 0x58
    table[0x59] = handle_return;          // Ret = 0x59
    table[0x5A] = handle_return_unit;     // RetV = 0x5A
    table[0x5B] = handle_call;            // Call = 0x5B
    table[0x5C] = handle_tail_call_op;    // TailCall = 0x5C
    table[0x5D] = handle_call_method;     // CallM = 0x5D
    table[0x5E] = handle_call_closure;    // CallClosure = 0x5E
    table[0x5F] = handle_call_indirect;   // CallR = 0x5F (indirect call via register)

    // ========================================================================
    // Memory + Collections (0x60-0x6F) - matches instruction.rs
    // ========================================================================
    table[0x60] = handle_new;             // New = 0x60
    table[0x61] = handle_new_generic;     // NewG = 0x61
    table[0x62] = handle_get_field;       // GetF = 0x62
    table[0x63] = handle_set_field;       // SetF = 0x63
    table[0x64] = handle_get_index;       // GetE = 0x64
    table[0x65] = handle_set_index;       // SetE = 0x65
    table[0x66] = handle_array_len;       // Len = 0x66
    table[0x67] = handle_new_array;       // NewArray = 0x67
    table[0x68] = handle_new_list;        // NewList = 0x68
    table[0x69] = handle_list_push;       // ListPush = 0x69
    table[0x6A] = handle_list_pop;        // ListPop = 0x6A
    table[0x6B] = handle_new_map;         // NewMap = 0x6B
    table[0x6C] = handle_map_get;         // MapGet = 0x6C
    table[0x6D] = handle_map_set;         // MapSet = 0x6D
    table[0x6E] = handle_map_contains;    // MapContains = 0x6E
    table[0x6F] = handle_clone;           // Clone = 0x6F

    // ========================================================================
    // CBGR Instructions (0x70-0x7F) - matches instruction.rs
    // ========================================================================
    table[0x70] = handle_ref_create;      // Ref = 0x70 (create immutable reference)
    table[0x71] = handle_ref_mut;         // RefMut = 0x71 (create mutable reference)
    table[0x72] = handle_deref;           // Deref = 0x72 (dereference)
    table[0x73] = handle_deref_mut;       // DerefMut = 0x73 (dereference mutable)
    table[0x74] = handle_chk_ref;         // ChkRef = 0x74 (CBGR validation check)
    table[0x75] = handle_ref_checked;     // RefChecked = 0x75 (create checked reference)
    table[0x76] = handle_ref_unsafe;      // RefUnsafe = 0x76 (create unsafe reference)
    table[0x77] = handle_drop_ref;        // DropRef = 0x77 (drop reference)
    table[0x78] = handle_cbgr_extended;   // CbgrExtended = 0x78 (CBGR extended ops)
    table[0x79] = handle_text_extended;   // TextExtended = 0x79 (text parsing/conversion ops)
    // 0x7A-0x7F: Reserved CBGR

    // ========================================================================
    // Generic + Variant (0x80-0x8F) - matches instruction.rs
    // ========================================================================
    table[0x80] = handle_call_generic;    // CallG = 0x80 (generic function call)
    table[0x81] = handle_call_virtual;    // CallV = 0x81 (virtual dispatch)
    table[0x82] = handle_call_cached;     // CallC = 0x82 (inline cached call)
    table[0x83] = handle_size_of;         // SizeOfG = 0x83 (size of generic type)
    table[0x84] = handle_align_of;        // AlignOfG = 0x84 (align of generic type)
    table[0x85] = handle_specialize;      // Instantiate = 0x85 (instantiate generic)
    table[0x86] = handle_make_variant;    // MakeVariant = 0x86 (create variant with tag)
    table[0x87] = handle_set_variant_data; // SetVariantData = 0x87
    table[0x88] = handle_get_variant_data; // GetVariantData = 0x88
    table[0x89] = handle_get_tag;         // GetTag = 0x89 (get variant/enum tag)
    table[0x8A] = handle_new_closure;     // NewClosure = 0x8A
    table[0x8B] = handle_get_variant_data_ref; // GetVariantDataRef = 0x8B (ref to field)
    table[0x8C] = handle_type_of;          // TypeOf = 0x8C (runtime type tag)
    table[0x8D] = handle_make_pi;          // MakePi = 0x8D (Π-value packaging)
    table[0x8E] = handle_make_sigma;       // MakeSigma = 0x8E (Σ-pair packaging)
    table[0x8F] = handle_make_witness;     // MakeWitness = 0x8F (refined + proof hash)

    // ========================================================================
    // Pattern Matching + Logic (0x90-0x9F) - matches instruction.rs
    // ========================================================================
    table[0x90] = handle_match_tag;       // IsVar = 0x90 (check variant)
    table[0x91] = handle_as_var;          // AsVar = 0x91 (extract variant payload)
    table[0x92] = handle_unpack;          // Unpack = 0x92 (unpack tuple)
    table[0x93] = handle_pack;              // Pack = 0x93 (pack into tuple)
    table[0x94] = handle_switch;          // Switch = 0x94 (switch/jump table)
    table[0x95] = handle_match_guard;     // MatchGuard = 0x95 (match guard check)
    table[0x96] = handle_land;            // And = 0x96 (logical AND)
    table[0x97] = handle_lor;             // Or = 0x97 (logical OR)
    table[0x98] = handle_lxor;            // Xor = 0x98 (logical XOR)
    table[0x99] = handle_lnot;            // Not = 0x99 (boolean not)
    // 0x9A-0x9F: Reserved pattern/logic

    // ========================================================================
    // Async + Nursery (0xA0-0xAF) - matches instruction.rs
    // ========================================================================
    table[0xA0] = handle_spawn;           // Spawn = 0xA0
    table[0xA1] = handle_await;           // Await = 0xA1
    table[0xA2] = handle_generator_yield; // Yield = 0xA2
    table[0xA3] = handle_select;          // Select = 0xA3
    table[0xA4] = handle_join;            // Join = 0xA4
    table[0xA5] = handle_future_ready;    // FutureReady = 0xA5
    table[0xA6] = handle_future_get;      // FutureGet = 0xA6
    table[0xA7] = handle_async_next;      // AsyncNext = 0xA7
    table[0xA8] = handle_nursery_init;    // NurseryInit = 0xA8
    table[0xA9] = handle_nursery_spawn;   // NurserySpawn = 0xA9
    table[0xAA] = handle_nursery_await;   // NurseryAwait = 0xAA
    table[0xAB] = handle_nursery_cancel;  // NurseryCancel = 0xAB
    table[0xAC] = handle_nursery_config;  // NurseryConfig = 0xAC
    table[0xAD] = handle_nursery_error;   // NurseryError = 0xAD
    // 0xAE-0xAF: Reserved async

    // ========================================================================
    // Context + Meta (0xB0-0xBF) - matches instruction.rs
    // ========================================================================
    table[0xB0] = handle_ctx_get;         // CtxGet = 0xB0
    table[0xB1] = handle_ctx_provide;     // CtxProvide = 0xB1
    table[0xB2] = handle_ctx_pop;         // CtxEnd = 0xB2
    table[0xB3] = handle_ctx_push;        // PushContext = 0xB3
    table[0xB4] = handle_ctx_pop;         // PopContext = 0xB4
    table[0xB5] = handle_attenuate;       // Attenuate = 0xB5
    table[0xB6] = handle_has_capability;  // HasCapability = 0xB6
    table[0xB7] = handle_require_capability; // RequireCapability = 0xB7

    // Meta Operations (0xB8-0xBB)
    table[0xB8] = handle_meta_eval;
    table[0xB9] = handle_meta_quote;
    table[0xBA] = handle_meta_splice;
    table[0xBB] = handle_meta_reflect;

    // FFI Extended (0xBC)
    table[0xBC] = handle_ffi_extended;

    // Arithmetic Extended (0xBD)
    table[0xBD] = handle_arith_extended;

    // Log Extended (0xBE) - Structured logging operations
    table[0xBE] = handle_log_extended;

    // Memory Extended (0xBF) - Heap allocation operations
    table[0xBF] = handle_mem_extended;

    // Iterator + Generator + String + Set ops (0xC0-0xCF)
    table[0xC0] = handle_iter_new;           // IterNew = 0xC0
    table[0xC1] = handle_iter_next;          // IterNext = 0xC1
    table[0xC2] = handle_generator_create;   // GenCreate = 0xC2
    table[0xC3] = handle_generator_next;     // GenNext = 0xC3
    table[0xC4] = handle_generator_has_next; // GenHasNext = 0xC4
    table[0xC5] = handle_to_string;          // ToString = 0xC5
    table[0xC6] = handle_concat;             // Concat = 0xC6
    table[0xC7] = handle_new_set;            // NewSet = 0xC7
    table[0xC8] = handle_set_insert;         // SetInsert = 0xC8
    table[0xC9] = handle_set_contains;       // SetContains = 0xC9
    table[0xCA] = handle_set_remove;         // SetRemove = 0xCA
    table[0xCB] = handle_char_to_str;        // CharToStr = 0xCB
    table[0xCC] = handle_new_range;          // NewRange = 0xCC
    table[0xCD] = handle_new_deque;          // NewDeque = 0xCD
    table[0xCE] = handle_push;               // Push = 0xCE
    table[0xCF] = handle_pop;                // Pop = 0xCF

    // Exception + Debug + Verify (0xD0-0xDF)
    table[0xD0] = handle_throw;              // Throw = 0xD0
    table[0xD1] = handle_try_begin;          // TryBegin = 0xD1
    table[0xD2] = handle_try_end;            // TryEnd = 0xD2
    table[0xD3] = handle_get_exception;      // GetException = 0xD3
    table[0xD4] = handle_spec;               // Spec = 0xD4
    table[0xD5] = handle_guard;              // Guard = 0xD5
    table[0xD6] = handle_assert;             // Assert = 0xD6
    table[0xD7] = handle_panic;              // Panic = 0xD7
    table[0xD8] = handle_unreachable;        // Unreachable = 0xD8
    table[0xD9] = handle_debug_print;        // DebugPrint = 0xD9
    table[0xDA] = handle_requires;           // Requires = 0xDA
    table[0xDB] = handle_ensures;            // Ensures = 0xDB
    table[0xDC] = handle_invariant;          // Invariant = 0xDC
    table[0xDD] = handle_new_channel;        // NewChannel = 0xDD
    table[0xDE] = handle_cubical_extended;  // CubicalExtended = 0xDE
    // DebugDF = 0xDF (reserved)

    // ========================================================================
    // System (V-LLSI) + Autodiff (0xE0-0xEF) - matches instruction.rs
    // ========================================================================
    table[0xE0] = handle_syscall;         // SyscallLinux = 0xE0
    table[0xE1] = handle_mmap;               // Mmap = 0xE1
    table[0xE2] = handle_munmap;             // Munmap = 0xE2
    table[0xE3] = handle_atomic_load;     // AtomicLoad = 0xE3
    table[0xE4] = handle_atomic_store;    // AtomicStore = 0xE4
    table[0xE5] = handle_atomic_cas;      // AtomicCas = 0xE5
    table[0xE6] = handle_atomic_fence;    // AtomicFence = 0xE6
    table[0xE7] = handle_io_submit;          // IoSubmit = 0xE7
    table[0xE8] = handle_io_poll;            // IoPoll = 0xE8
    table[0xE9] = handle_tls_get;         // TlsGet = 0xE9
    table[0xEA] = handle_tls_set;         // TlsSet = 0xEA
    table[0xEB] = handle_grad_begin;      // GradBegin = 0xEB
    table[0xEC] = handle_grad_end;        // GradEnd = 0xEC
    table[0xED] = handle_grad_checkpoint; // GradCheckpoint = 0xED
    table[0xEE] = handle_grad_accumulate; // GradAccumulate = 0xEE
    table[0xEF] = handle_grad_stop;       // GradStop = 0xEF

    // ========================================================================
    // Tensor Operations (0xF0-0xF7) - matches instruction.rs
    // ========================================================================
    table[0xF0] = handle_tensor_new;          // TensorNew = 0xF0
    table[0xF1] = handle_tensor_binop;        // TensorBinop = 0xF1
    table[0xF2] = handle_tensor_unop;         // TensorUnop = 0xF2
    table[0xF3] = handle_tensor_matmul;       // TensorMatmul = 0xF3
    table[0xF4] = handle_tensor_reduce;       // TensorReduce = 0xF4
    table[0xF5] = handle_tensor_reshape;      // TensorReshape = 0xF5
    table[0xF6] = handle_tensor_transpose;    // TensorTranspose = 0xF6
    table[0xF7] = handle_tensor_slice;        // TensorSlice = 0xF7

    // GPU Operations (0xF8-0xFB)
    table[0xF8] = handle_gpu_extended;       // GpuExtended = 0xF8
    table[0xF9] = handle_gpu_sync;           // GpuSync = 0xF9
    table[0xFA] = handle_gpu_memcpy;         // GpuMemcpy = 0xFA
    table[0xFB] = handle_gpu_alloc;          // GpuAlloc = 0xFB

    // Tensor Extended Opcode (0xFC)
    table[0xFC] = handle_tensor_extended;    // TensorExtended = 0xFC

    // ML Extended Opcode (0xFD)
    table[0xFD] = handle_ml_extended;        // MlExtended = 0xFD

    // Tensor Additional (0xFE-0xFF)
    table[0xFE] = handle_tensor_full;        // TensorFull = 0xFE
    table[0xFF] = handle_tensor_from_slice;  // TensorFromSlice = 0xFF

    table
}

// ============================================================================
// Shared Utility Functions
// ============================================================================
//
// These functions are used by multiple handler modules and remain here as
// the single source of truth. Handler modules access them via super::super::.

/// Process a function return: pop frame, record stats, handle awaited tasks.
///
/// This is the core return-handling logic used by Ret/RetV handlers and
/// by the dispatch loop for implicit returns at end of function.
pub(crate) fn do_return(state: &mut InterpreterState, value: Value) -> InterpreterResult<DispatchResult> {
    // If returning from a generator function, mark it as Completed
    if let Some(gen_id) = state.current_generator.take() {
        state.generators.complete(gen_id, Some(value));
    }

    // Pop current frame
    let frame = match state.call_stack.pop_frame() {
        Ok(f) => f,
        Err(_) => {
            // Stack underflow - this is the final return from main
            return Ok(DispatchResult::FinalReturn(value));
        }
    };

    // Pop registers
    state.registers.pop_frame(frame.reg_base);

    // Record return in stats
    state.record_return();

    // Check if we're returning from an awaited task and mark it as Completed
    if let Some(task_id) = state.awaiting_task.take() {
        state.tasks.complete(task_id, value);
    }

    // If stack is now empty, return the value (final return from main)
    if state.call_stack.is_empty() {
        return Ok(DispatchResult::FinalReturn(value));
    }

    // Store return value in caller's register
    let caller_base = frame.caller_base;
    state.registers.set(caller_base, frame.return_reg, value);

    // Restore caller's PC
    state.set_pc(frame.return_pc);

    // Continue execution in caller
    Ok(DispatchResult::Continue)
}

/// Load a constant from the constant pool and produce a Value.
pub(crate) fn load_constant(state: &mut InterpreterState, const_id: ConstId) -> InterpreterResult<Value> {
    // Clone constant to release borrow before allocation
    let constant = state
        .module
        .get_constant(const_id)
        .cloned()
        .ok_or_else(|| InterpreterError::InvalidBytecode {
            pc: state.pc() as usize,
            message: format!("Invalid constant id: {:?}", const_id),
        })?;

    match constant {
        Constant::Int(i) => Ok(Value::from_i64(i)),
        Constant::Float(f) => Ok(Value::from_f64(f)),
        Constant::String(string_id) => {
            if let Some(s) = state.module.get_string(string_id) {
                let s = s.to_string();
                if let Some(small) = Value::from_small_string(&s) {
                    Ok(small)
                } else {
                    // Allocate on heap: [len: u64][bytes...]
                    let bytes = s.as_bytes();
                    let len = bytes.len();
                    let alloc_size = 8 + len;
                    let obj = state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?;
                    state.record_allocation();
                    let base_ptr = obj.as_ptr() as *mut u8;
                    // SAFETY: obj was just allocated with size 8 + len. Writing the length
                    // as u64 at data_offset, then copying `len` bytes of string data after it.
                    unsafe {
                        let data_offset = super::heap::OBJECT_HEADER_SIZE;
                        let len_ptr = base_ptr.add(data_offset) as *mut u64;
                        *len_ptr = len as u64;
                        let bytes_ptr = base_ptr.add(data_offset + 8);
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
                    }
                    Ok(Value::from_ptr(base_ptr))
                }
            } else {
                Ok(Value::from_small_string("").unwrap_or(Value::nil()))
            }
        }
        Constant::Type(type_ref) => {
            match type_ref {
                crate::types::TypeRef::Concrete(type_id) => Ok(Value::from_type(type_id)),
                _ => Ok(Value::nil()),
            }
        }
        Constant::Function(func_id) => Ok(Value::from_function(func_id)),
        Constant::Protocol(_) => Ok(Value::nil()),
        Constant::Array(ref element_ids) => {
            let count = element_ids.len();
            let elem_ids: Vec<ConstId> = element_ids.clone();
            let mut elements = Vec::with_capacity(count);
            for elem_id in elem_ids.iter() {
                let elem_val = load_constant(state, *elem_id)?;
                elements.push(elem_val);
            }
            let header_size = 3 * std::mem::size_of::<i64>();
            let obj = state.heap.alloc(TypeId::LIST, header_size)?;
            state.record_allocation();
            // SAFETY: obj was allocated with header_size bytes of data space. We write
            // the list header (len, cap, backing_ptr) as three i64 values.
            let data_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(super::heap::OBJECT_HEADER_SIZE) as *mut i64
            };
            let backing_layout = std::alloc::Layout::from_size_align(
                count.max(1) * std::mem::size_of::<Value>(), 8
            ).map_err(|_| InterpreterError::Panic {
                message: "array constant layout overflow".into(),
            })?;
            // SAFETY: Layout was validated above via from_size_align.
            let backing_ptr = unsafe { std::alloc::alloc_zeroed(backing_layout) };
            if backing_ptr.is_null() && count > 0 {
                return Err(InterpreterError::Panic {
                    message: "array constant allocation failed".into(),
                });
            }
            let value_ptr = backing_ptr as *mut Value;
            for (i, val) in elements.iter().enumerate() {
                // SAFETY: backing buffer has space for `count` Values, and i < count.
                unsafe { *value_ptr.add(i) = *val };
            }
            // SAFETY: data_ptr points to the list header with room for 3 i64 values.
            unsafe {
                *data_ptr = count as i64;
                *data_ptr.add(1) = count as i64;
                *data_ptr.add(2) = backing_ptr as i64;
            }
            Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
        }
        Constant::Bytes(bytes) => {
            let obj = state.heap.alloc(TypeId::U8, bytes.len())?;
            // SAFETY: obj was allocated with bytes.len() data space. Copying the exact
            // number of bytes into the data region is valid.
            unsafe {
                let data_ptr = (obj.as_ptr() as *mut u8).add(super::heap::OBJECT_HEADER_SIZE);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, bytes.len());
            }
            Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
        }
    }
}

/// Call a closure synchronously, returning its result.
/// Sets up a call frame for the closure and runs a nested dispatch loop.
pub(crate) fn call_closure_sync(
    state: &mut InterpreterState,
    closure_val: Value,
    args: &[Value],
) -> InterpreterResult<Value> {
    if !closure_val.is_ptr() || closure_val.is_nil() {
        return Err(InterpreterError::TypeMismatch {
            expected: "closure",
            got: "non-pointer",
            operation: "call_closure_sync",
        });
    }

    let base_ptr = closure_val.as_ptr::<u8>();
    let header_offset = super::heap::OBJECT_HEADER_SIZE;

    // SAFETY: Closure objects are allocated with layout [header | func_id: u32 | capture_count: u32 | captures...].
    // base_ptr + header_offset points to the func_id field.
    let (func_id, capture_count) = unsafe {
        let func_id = *(base_ptr.add(header_offset) as *const u32);
        let capture_count = *(base_ptr.add(header_offset + 4) as *const u32);
        (FunctionId(func_id), capture_count as usize)
    };

    let func = state
        .module
        .get_function(func_id)
        .ok_or({
            InterpreterError::FunctionNotFound(func_id)
        })?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let entry_depth = state.call_stack.depth();

    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
    state.registers.try_push_frame(reg_count).map_err(|new_top| {
        InterpreterError::StackOverflow {
            depth: new_top,
            max_depth: crate::interpreter::registers::MAX_SIZE,
        }
    })?;

    // Copy captured values
    // SAFETY: Closure layout guarantees captures_offset (header + 8) followed by
    // capture_count Values. Each read is within the allocated closure object.
    unsafe {
        let captures_offset = header_offset + 8;
        for i in 0..capture_count {
            let cap_ptr = base_ptr.add(captures_offset + i * std::mem::size_of::<Value>()) as *const Value;
            state.registers.set(new_base, Reg(i as u16), std::ptr::read(cap_ptr));
        }
    }

    // Copy arguments after captures
    for (i, val) in args.iter().enumerate() {
        state.registers.set(new_base, Reg((capture_count + i) as u16), *val);
    }

    state.set_pc(0);
    dispatch_loop_table_with_entry_depth(state, entry_depth)
}

/// Execute a function by FunctionId synchronously, returning its result.
fn call_function_sync(
    state: &mut InterpreterState,
    func_id: FunctionId,
    args: &[Value],
) -> InterpreterResult<Value> {
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let entry_depth = state.call_stack.depth();

    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
    state.registers.try_push_frame(reg_count).map_err(|new_top| {
        InterpreterError::StackOverflow {
            depth: new_top,
            max_depth: crate::interpreter::registers::MAX_SIZE,
        }
    })?;

    for (i, val) in args.iter().enumerate() {
        state.registers.set(new_base, Reg(i as u16), *val);
    }

    state.set_pc(0);
    dispatch_loop_table_with_entry_depth(state, entry_depth)
}

/// Execute a pending task from the task queue.
pub(crate) fn execute_pending_task(state: &mut InterpreterState, task_id: TaskId) -> InterpreterResult<()> {
    let exec_info = state.tasks.take_task_exec_info(task_id);

    if let Some((func_id, args, closure_val, saved_contexts)) = exec_info {
        // Restore parent's context stack into the child task.
        // This ensures spawned tasks inherit contexts from their parent.
        if !saved_contexts.is_empty() {
            for entry in &saved_contexts {
                state.context_stack.provide(entry.ctx_type, entry.value, 0);
            }
        }

        let result = if let Some(closure) = closure_val {
            call_closure_sync(state, closure, &args)
        } else {
            call_function_sync(state, func_id, &args)
        };

        match result {
            Ok(value) => {
                state.tasks.complete(task_id, value);
            }
            Err(e) => {
                state.tasks.fail(task_id);
                return Err(e);
            }
        }
    }

    Ok(())
}

/// Allocate a new List from a Vec of Values, returning a pointer Value.
pub(crate) fn alloc_list_from_values(state: &mut InterpreterState, values: Vec<Value>) -> InterpreterResult<Value> {
    let len = values.len();
    let cap = len.max(1);

    let obj = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let backing = state.heap.alloc_array(TypeId::LIST, cap)?;
    state.record_allocation();

    // SAFETY: backing was allocated with space for `cap` Values after the header.
    let backing_data = unsafe {
        (backing.as_ptr() as *mut u8).add(super::heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for (i, val) in values.into_iter().enumerate() {
        // SAFETY: i < len <= cap, so backing_data.add(i) is within the allocation.
        unsafe { *backing_data.add(i) = val };
    }

    // SAFETY: obj was allocated with 3 * sizeof(Value) data space for the list header.
    let data_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(super::heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    // SAFETY: Writing list header fields (len, cap, backing_ptr) into the 3-Value data region.
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }

    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

// ============================================================================
// Deep Value Equality (used by method_dispatch and comparison handlers)
// ============================================================================

/// Get the length of an array (Value array or List).
pub(crate) fn get_array_length(ptr: *const u8, header: &super::heap::ObjectHeader) -> InterpreterResult<usize> {
    if header.type_id == TypeId::LIST {
        // SAFETY: List objects have a 3-Value header [len, cap, backing_ptr] after OBJECT_HEADER_SIZE.
        // The first Value is the length.
        let data_ptr = unsafe { ptr.add(super::heap::OBJECT_HEADER_SIZE) as *const Value };
        Ok(unsafe { (*data_ptr).as_i64() } as usize)
    } else {
        Ok(header.size as usize / std::mem::size_of::<Value>())
    }
}

/// Get element at index from an array (Value array or List).
///
/// SECURITY: `index * size_of::<Value>()` can overflow `usize` on
/// huge indices, producing a wrapped offset that would point into
/// arbitrary memory.  Use `checked_mul` and return an overflow
/// error if the multiplication wraps.  This was previously a
/// silent unsafe-multiply hazard duplicated across handlers; this
/// canonical implementation absorbs the safer variant.
pub(crate) fn get_array_element(
    ptr: *const u8,
    header: &super::heap::ObjectHeader,
    index: usize,
) -> InterpreterResult<Value> {
    let elem_offset = index
        .checked_mul(std::mem::size_of::<Value>())
        .ok_or(InterpreterError::IntegerOverflow {
            operation: "array_index_offset",
        })?;

    if header.type_id == TypeId::LIST {
        // SAFETY: List layout is [header | len | cap | backing_ptr]. The backing pointer
        // points to an array allocation with elements after its own OBJECT_HEADER_SIZE.
        // `elem_offset` was checked_mul-bounded above, so the pointer arithmetic stays
        // within `usize` and the resulting address is within the live allocation.
        let data_ptr = unsafe { ptr.add(super::heap::OBJECT_HEADER_SIZE) as *const Value };
        let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
        let elem_ptr =
            unsafe { backing.add(super::heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        Ok(unsafe { *elem_ptr })
    } else {
        // SAFETY: Non-LIST arrays store Values directly after the header.  Same
        // checked-offset reasoning as the LIST branch above.
        let elem_ptr =
            unsafe { ptr.add(super::heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        Ok(unsafe { *elem_ptr })
    }
}

/// Check if a type_id represents an array type.
fn is_array_type_id(type_id: u32) -> bool {
    type_id == 0
        || type_id == TypeId::LIST.0
        || type_id == TypeId::ARRAY.0
        || type_id == TypeId::TUPLE.0
}

/// Extract a string representation from a Value.
fn extract_string(value: &Value, _state: &InterpreterState) -> String {
    if value.is_small_string() {
        return value.as_small_string().as_str().to_string();
    }

    if value.is_ptr() && !value.is_nil() {
        let base_ptr = value.as_ptr::<u8>();
        if !base_ptr.is_null() {
            unsafe {
                let data_offset = super::heap::OBJECT_HEADER_SIZE;
                let len_ptr = base_ptr.add(data_offset) as *const u64;
                let len = *len_ptr as usize;
                if len <= 65536 {
                    let bytes_ptr = base_ptr.add(data_offset + 8);
                    let bytes = std::slice::from_raw_parts(bytes_ptr, len);
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        return s.to_string();
                    }
                }
            }
        }
    }

    if value.is_int() {
        return format!("{}", value.as_i64());
    }

    format!("<value:{}>", value.as_i64())
}

/// Check if a Value is a heap-allocated string pointer.
fn is_heap_string(v: &Value) -> bool {
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return false;
    }
    let type_id = unsafe { *(ptr as *const u32) };
    type_id == 0x0001 || type_id == TypeId::TEXT.0
}

/// Check if an integer Value might be a string table index.
fn is_string_id(v: &Value, state: &InterpreterState) -> bool {
    if !v.is_int() || v.is_bool() {
        return false;
    }
    let id = v.as_i64();
    id >= 0 && (id as u32) < 10000 && state.module.get_string(crate::types::StringId(id as u32)).is_some()
}

/// Resolve a Value to its string content.
fn resolve_string_value(v: &Value, state: &InterpreterState) -> String {
    if v.is_small_string() {
        return v.as_small_string().as_str().to_string();
    }
    if is_heap_string(v) {
        let base_ptr = v.as_ptr::<u8>();
        unsafe {
            let data_offset = super::heap::OBJECT_HEADER_SIZE;
            let len_ptr = base_ptr.add(data_offset) as *const u64;
            let len = *len_ptr as usize;
            if len <= 65536 {
                let bytes_ptr = base_ptr.add(data_offset + 8);
                let bytes = std::slice::from_raw_parts(bytes_ptr, len);
                if let Ok(s) = std::str::from_utf8(bytes) {
                    return s.to_string();
                }
            }
        }
    }
    if v.is_int() && !v.is_bool() {
        let id = v.as_i64();
        if id >= 0
            && let Some(s) = state.module.get_string(crate::types::StringId(id as u32)) {
                return s.to_string();
            }
    }
    format!("<value:{}>", v.as_i64())
}

/// CBGR reference encoding helpers (for deep_value_eq).
fn is_cbgr_ref(val: &Value) -> bool {
    if !val.is_int() || val.is_bool() {
        return false;
    }
    let bits = val.as_i64();
    let sentinel = bits & !0xFFFF_FFFF_FFFFi64;
    sentinel == -0x4000_0000_0000_0000i64 || sentinel == -0x3000_0000_0000_0000i64
}

fn decode_cbgr_ref(encoded: i64) -> (u32, u32) {
    let abs_index = (encoded & 0xFFFF) as u32;
    let generation = ((encoded >> 16) & 0xFFFF) as u32;
    (abs_index, generation)
}

/// Deep structural value equality comparison.
///
/// Handles all value types including heap objects (variants, tuples, arrays),
/// references (ThinRef, FatRef, CBGR), and cross-representation strings.
pub(crate) fn deep_value_eq(va: &Value, vb: &Value, state: &InterpreterState) -> bool {
    use super::heap::OBJECT_HEADER_SIZE;

    // IEEE 754: NaN is never equal to anything
    const TAG_NAN: u8 = 0x7;
    if va.tag() == Some(TAG_NAN) || vb.tag() == Some(TAG_NAN) {
        return false;
    }
    if va.is_float() && va.as_f64().is_nan() {
        return false;
    }
    if vb.is_float() && vb.as_f64().is_nan() {
        return false;
    }

    // Fast path: identical bit patterns
    if va.to_bits() == vb.to_bits() {
        return true;
    }

    // ThinRef comparison
    if va.is_thin_ref() && vb.is_thin_ref() {
        let thin_a = va.as_thin_ref();
        let thin_b = vb.as_thin_ref();
        if thin_a.is_null() && thin_b.is_null() {
            return true;
        }
        if thin_a.is_null() || thin_b.is_null() {
            return false;
        }
        let deref_a = unsafe { *(thin_a.ptr as *const Value) };
        let deref_b = unsafe { *(thin_b.ptr as *const Value) };
        return deep_value_eq(&deref_a, &deref_b, state);
    }

    // FatRef comparison
    if va.is_fat_ref() && vb.is_fat_ref() {
        let fat_a = va.as_fat_ref();
        let fat_b = vb.as_fat_ref();
        if fat_a.is_null() && fat_b.is_null() {
            return true;
        }
        if fat_a.is_null() || fat_b.is_null() {
            return false;
        }
        let deref_a = unsafe { *(fat_a.ptr() as *const Value) };
        let deref_b = unsafe { *(fat_b.ptr() as *const Value) };
        return deep_value_eq(&deref_a, &deref_b, state);
    }

    // Mixed: ThinRef vs CBGR register ref
    if va.is_thin_ref() && is_cbgr_ref(vb) {
        let thin_a = va.as_thin_ref();
        if thin_a.is_null() {
            return false;
        }
        let deref_a = unsafe { *(thin_a.ptr as *const Value) };
        let (abs_idx, _gen) = decode_cbgr_ref(vb.as_i64());
        let deref_b = state.registers.get_absolute(abs_idx);
        return deep_value_eq(&deref_a, &deref_b, state);
    }

    if is_cbgr_ref(va) && vb.is_thin_ref() {
        let thin_b = vb.as_thin_ref();
        if thin_b.is_null() {
            return false;
        }
        let (abs_idx, _gen) = decode_cbgr_ref(va.as_i64());
        let deref_a = state.registers.get_absolute(abs_idx);
        let deref_b = unsafe { *(thin_b.ptr as *const Value) };
        return deep_value_eq(&deref_a, &deref_b, state);
    }

    // Two CBGR register references
    if is_cbgr_ref(va) && is_cbgr_ref(vb) {
        let (abs_idx_a, _gen_a) = decode_cbgr_ref(va.as_i64());
        let (abs_idx_b, _gen_b) = decode_cbgr_ref(vb.as_i64());
        let deref_a = state.registers.get_absolute(abs_idx_a);
        let deref_b = state.registers.get_absolute(abs_idx_b);
        return deep_value_eq(&deref_a, &deref_b, state);
    }

    // Primitives
    if va.is_float() && vb.is_float() {
        return va.as_f64() == vb.as_f64();
    }
    if va.is_bool() && vb.is_bool() {
        return va.as_bool() == vb.as_bool();
    }
    // Bool <-> Int cross-type comparison
    if (va.is_bool() && vb.is_int()) || (va.is_int() && vb.is_bool()) {
        let ia = if va.is_bool() { va.as_bool() as i64 } else { va.as_i64() };
        let ib = if vb.is_bool() { vb.as_bool() as i64 } else { vb.as_i64() };
        return ia == ib;
    }
    if va.is_unit() && vb.is_unit() {
        return true;
    }
    if va.is_nil() && vb.is_nil() {
        return true;
    }

    // String comparison (cross-representation)
    let a_is_string_like = va.is_small_string() || is_heap_string(va) || is_string_id(va, state);
    let b_is_string_like = vb.is_small_string() || is_heap_string(vb) || is_string_id(vb, state);

    if a_is_string_like && b_is_string_like {
        let str_a = resolve_string_value(va, state);
        let str_b = resolve_string_value(vb, state);
        return str_a == str_b;
    }

    // Pure integer comparison
    if va.is_int() && vb.is_int() {
        return va.as_i64() == vb.as_i64();
    }

    // Small string vs non-string
    if va.is_small_string() || vb.is_small_string() {
        return false;
    }

    // Heap pointer comparison (variants, tuples, arrays, strings)
    if va.is_ptr() && vb.is_ptr() {
        let ptr_a = va.as_ptr::<u8>();
        let ptr_b = vb.as_ptr::<u8>();

        if ptr_a.is_null() && ptr_b.is_null() {
            return true;
        }
        if ptr_a.is_null() || ptr_b.is_null() {
            return false;
        }

        let type_id_a = unsafe { *(ptr_a as *const u32) };
        let type_id_b = unsafe { *(ptr_b as *const u32) };

        // Variant comparison
        if type_id_a >= 0x8000 && type_id_b >= 0x8000 {
            let tag_a = unsafe { *(ptr_a.add(OBJECT_HEADER_SIZE) as *const u32) };
            let tag_b = unsafe { *(ptr_b.add(OBJECT_HEADER_SIZE) as *const u32) };

            if tag_a != tag_b {
                return false;
            }

            let header_a = unsafe { &*(ptr_a as *const super::heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const super::heap::ObjectHeader) };
            let size_a = header_a.size as usize;
            let size_b = header_b.size as usize;
            if size_a != size_b {
                return false;
            }
            let field_count = size_a.saturating_sub(8) / std::mem::size_of::<Value>();
            let payload_offset = OBJECT_HEADER_SIZE + 8;
            for i in 0..field_count {
                let fa = unsafe { &*(ptr_a.add(payload_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                let fb = unsafe { &*(ptr_b.add(payload_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                if !deep_value_eq(fa, fb, state) {
                    return false;
                }
            }
            return true;
        } else if (type_id_a == 0 || type_id_a == TypeId::TUPLE.0) && (type_id_b == 0 || type_id_b == TypeId::TUPLE.0) {
            // Tuple/pack comparison
            let header_a = unsafe { &*(ptr_a as *const super::heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const super::heap::ObjectHeader) };
            let size_a = header_a.size as usize;
            let size_b = header_b.size as usize;
            if size_a != size_b {
                return false;
            }
            let field_count = size_a / std::mem::size_of::<Value>();
            let data_offset = OBJECT_HEADER_SIZE;
            for i in 0..field_count {
                let fa = unsafe { &*(ptr_a.add(data_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                let fb = unsafe { &*(ptr_b.add(data_offset + i * std::mem::size_of::<Value>()) as *const Value) };
                if !deep_value_eq(fa, fb, state) {
                    return false;
                }
            }
            return true;
        } else if is_array_type_id(type_id_a) && is_array_type_id(type_id_b) {
            // Array/List structural comparison
            let header_a = unsafe { &*(ptr_a as *const super::heap::ObjectHeader) };
            let header_b = unsafe { &*(ptr_b as *const super::heap::ObjectHeader) };
            let len_a = get_array_length(ptr_a, header_a).unwrap_or(0);
            let len_b = get_array_length(ptr_b, header_b).unwrap_or(0);
            if len_a != len_b {
                return false;
            }
            for i in 0..len_a {
                let ea = get_array_element(ptr_a, header_a, i).unwrap_or(Value::nil());
                let eb = get_array_element(ptr_b, header_b, i).unwrap_or(Value::nil());
                if !deep_value_eq(&ea, &eb, state) {
                    return false;
                }
            }
            return true;
        } else if type_id_a == type_id_b {
            // Same type - likely strings
            let str_a = extract_string(va, state);
            let str_b = extract_string(vb, state);
            return str_a == str_b;
        } else {
            return false;
        }
    }

    // Type mismatch
    false
}

/// Default handler for unimplemented opcodes.
fn handle_not_implemented(_state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    Err(InterpreterError::NotImplemented {
        feature: "unknown opcode",
        opcode: None,
    })
}

// ============================================================================
// Optimized Dispatch Loop
// ============================================================================

/// Optimized dispatch loop using function table.
///
/// This is ~30-50% faster than match-based dispatch for hot loops
/// due to better branch prediction and reduced code size.
pub fn dispatch_loop_table(state: &mut InterpreterState) -> InterpreterResult<Value> {
    dispatch_loop_table_with_entry_depth(state, 0)
}

/// Dispatch loop for nested/callback execution.
///
/// This variant tracks the entry stack depth and returns when the stack depth
/// falls back to the entry level after a return. This is essential for re-entrant
/// execution (e.g., FFI callbacks, closure calls) where we want to return control
/// to the caller after the nested function completes.
pub fn dispatch_loop_table_with_entry_depth(
    state: &mut InterpreterState,
    entry_depth: usize,
) -> InterpreterResult<Value> {
    // Capture wall-clock start when `timeout_ms` is configured.
    // Sampled every 256 instructions to bound the cost of
    // `Instant::now()` (~50ns) — matches the existing
    // cancel-flag cadence. Closes the inert-defense pattern:
    // prior to wiring, `InterpreterConfig.timeout_ms` was
    // declared, defaulted, and never read; adversarial bytecode
    // could spin past the documented budget without the
    // dispatch loop ever sampling time.
    let timeout_deadline: Option<std::time::Instant> = if state.config.timeout_ms > 0 {
        Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(state.config.timeout_ms),
        )
    } else {
        None
    };

    loop {
        state.global_instruction_count += 1;
        // Cooperative cancellation: check external flag every 256 instructions (~2.5μs)
        if state.global_instruction_count & 0xFF == 0 {
            if let Some(ref flag) = state.config.cancel_flag
                && flag.load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(InterpreterError::InstructionLimitExceeded {
                    count: state.global_instruction_count,
                    limit: 0,
                });
            }
            // Wall-clock timeout: surface as
            // `InstructionLimitExceeded` with `limit = 0` since
            // the time-based ceiling has no instruction-count
            // analogue. The interpreter API uses one fail-closed
            // error shape for any "abort because budget
            // exhausted" case — consistent triage in the caller.
            if let Some(deadline) = timeout_deadline
                && std::time::Instant::now() >= deadline
            {
                return Err(InterpreterError::InstructionLimitExceeded {
                    count: state.global_instruction_count,
                    limit: 0,
                });
            }
        }
        if state.global_instruction_count > state.config.max_instructions && state.config.max_instructions > 0 {
            return Err(InterpreterError::InstructionLimitExceeded {
                count: state.global_instruction_count,
                limit: state.config.max_instructions,
            });
        }
        // Check if we have bytecode
        let bytecode = match state.current_bytecode() {
            Some(bc) => bc,
            None => return Ok(Value::unit()),
        };

        let pc = state.pc() as usize;

        // End of function check - handle implicit return unit
        if pc >= bytecode.len() {
            match do_return(state, Value::unit())? {
                DispatchResult::FinalReturn(value) => return Ok(value),
                DispatchResult::Continue => {
                    if entry_depth > 0 && state.call_stack.depth() <= entry_depth {
                        return Ok(Value::unit());
                    }
                    continue;
                }
                DispatchResult::Return(value) => {
                    if entry_depth > 0 && state.call_stack.depth() <= entry_depth {
                        return Ok(value);
                    }
                    continue;
                }
                DispatchResult::Yield(value) => return Ok(value),
            }
        }

        // Fetch opcode
        let opcode_byte = bytecode[pc];
        state.advance_pc(1);
        state.record_instruction();


        // Dispatch via table lookup - O(1) array indexing
        let handler = DISPATCH_TABLE[opcode_byte as usize];
        let result = handler(state).map_err(|e| {
            match e {
                InterpreterError::NotImplemented { feature: _, opcode: None } => {
                    InterpreterError::InvalidOpcode {
                        opcode: opcode_byte,
                        pc,
                    }
                }
                // Enrich bare `NullPointer` with the PC and opcode so the
                // developer gets enough context to map the error back to
                // a specific instruction. Without this every null deref
                // surfaces as a single line "Null pointer dereference"
                // with no way to locate the failing site.
                InterpreterError::NullPointer => {
                    let site = state
                        .call_stack
                        .current_function_name(&state.module)
                        .unwrap_or_else(|| "<unknown>".to_string());
                    InterpreterError::NullPointerAt {
                        op: format!("opcode 0x{:02x}", opcode_byte),
                        site,
                        pc: pc as u32,
                    }
                }
                other => other,
            }
        })?;
        match result {
            DispatchResult::Continue => {
                if entry_depth > 0 && state.call_stack.depth() <= entry_depth {
                    let ret_val = state.get_reg(Reg(0));
                    return Ok(ret_val);
                }
                continue;
            }
            DispatchResult::Return(value) => {
                if entry_depth > 0 && state.call_stack.depth() <= entry_depth {
                    return Ok(value);
                }
                continue;
            }
            DispatchResult::FinalReturn(value) => {
                return Ok(value);
            }
            DispatchResult::Yield(value) => {
                return Ok(value);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode;
    use crate::instruction::{Instruction, ArithSubOpcode};
    use crate::module::{FunctionDescriptor, VbcModule};
    use crate::types::StringId;
    use std::sync::Arc;

    #[test]
    fn test_dispatch_table_size() {
        assert_eq!(DISPATCH_TABLE.len(), 256);
    }

    #[test]
    fn test_handler_coverage() {
        let not_impl_ptr = handle_not_implemented as *const () as usize;
        assert!(DISPATCH_TABLE[0x00] as *const () as usize != not_impl_ptr); // Mov
        assert!(DISPATCH_TABLE[0x10] as *const () as usize != not_impl_ptr); // AddI
        assert!(DISPATCH_TABLE[0x50] as *const () as usize != not_impl_ptr); // Jump
    }

    #[test]
    fn test_arith_extended_handler_registered() {
        let not_impl_ptr = handle_not_implemented as *const () as usize;
        assert!(DISPATCH_TABLE[0xBD] as *const () as usize != not_impl_ptr);
    }

    fn create_test_module(bytecode: Vec<u8>, register_count: u16) -> Arc<VbcModule> {
        let mut module = VbcModule::new("arith_test".to_string());

        let mut func = FunctionDescriptor::new(StringId::EMPTY);
        func.id = FunctionId(0);
        func.bytecode_offset = 0;
        func.bytecode_length = bytecode.len() as u32;
        func.register_count = register_count;

        module.functions.push(func);
        module.bytecode = bytecode;

        Arc::new(module)
    }

    fn encode_instructions(instructions: &[Instruction]) -> Vec<u8> {
        let mut bc = Vec::new();
        for instr in instructions {
            bytecode::encode_instruction(instr, &mut bc);
        }
        bc
    }

    #[test]
    fn test_poly_add_integers() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 10 },
            Instruction::LoadSmallI { dst: Reg(1), value: 20 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyAdd.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), 30);
    }

    #[test]
    fn test_poly_add_floats() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 1.5 },
            Instruction::LoadF { dst: Reg(1), value: 2.5 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyAdd.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 4.0).abs() < 0.0001);
    }

    #[test]
    fn test_poly_sub_integers() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 100 },
            Instruction::LoadSmallI { dst: Reg(1), value: 30 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolySub.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), 70);
    }

    #[test]
    fn test_poly_mul_integers() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 7 },
            Instruction::LoadSmallI { dst: Reg(1), value: 8 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyMul.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), 56);
    }

    #[test]
    fn test_poly_div_integers() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 100 },
            Instruction::LoadSmallI { dst: Reg(1), value: 5 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyDiv.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), 20);
    }

    #[test]
    fn test_poly_div_by_zero() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 100 },
            Instruction::LoadSmallI { dst: Reg(1), value: 0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyDiv.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state);
        assert!(matches!(result, Err(InterpreterError::DivisionByZero)));
    }

    #[test]
    fn test_poly_neg_integer() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 42 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyNeg.to_byte(),
                operands: vec![1, 0],
            },
            Instruction::Ret { value: Reg(1) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), -42);
    }

    #[test]
    fn test_poly_rem_integers() {
        let bytecode = encode_instructions(&[
            Instruction::LoadSmallI { dst: Reg(0), value: 17 },
            Instruction::LoadSmallI { dst: Reg(1), value: 5 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::PolyRem.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert_eq!(result.as_i64(), 2);
    }

    // ========================================================================
    // Binary Float Operations Tests
    // ========================================================================

    #[test]
    fn test_atan2() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 1.0 },
            Instruction::LoadF { dst: Reg(1), value: 1.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Atan2.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        let expected = std::f64::consts::FRAC_PI_4;
        assert!((result.as_f64() - expected).abs() < 1e-10);
    }

    #[test]
    fn test_atan2_negative() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: -1.0 },
            Instruction::LoadF { dst: Reg(1), value: -1.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Atan2.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        let expected = -3.0 * std::f64::consts::FRAC_PI_4;
        assert!((result.as_f64() - expected).abs() < 1e-10);
    }

    #[test]
    fn test_hypot() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 3.0 },
            Instruction::LoadF { dst: Reg(1), value: 4.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Hypot.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_copysign_positive_to_negative() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 5.0 },
            Instruction::LoadF { dst: Reg(1), value: -1.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Copysign.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - (-5.0)).abs() < 1e-10);
    }

    #[test]
    fn test_copysign_negative_to_positive() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: -5.0 },
            Instruction::LoadF { dst: Reg(1), value: 1.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Copysign.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_pow() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 2.0 },
            Instruction::LoadF { dst: Reg(1), value: 3.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Pow.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_pow_fractional() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 4.0 },
            Instruction::LoadF { dst: Reg(1), value: 0.5 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Pow.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_log_base() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 8.0 },
            Instruction::LoadF { dst: Reg(1), value: 2.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::LogBase.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_fmod() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 5.5 },
            Instruction::LoadF { dst: Reg(1), value: 2.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Fmod.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_fmod_negative() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: -5.5 },
            Instruction::LoadF { dst: Reg(1), value: 2.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Fmod.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - (-1.5)).abs() < 1e-10);
    }

    #[test]
    fn test_remainder() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 5.5 },
            Instruction::LoadF { dst: Reg(1), value: 2.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Remainder.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - (-0.5)).abs() < 1e-10);
    }

    #[test]
    fn test_fdim_positive() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 5.0 },
            Instruction::LoadF { dst: Reg(1), value: 3.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Fdim.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_fdim_zero() {
        let bytecode = encode_instructions(&[
            Instruction::LoadF { dst: Reg(0), value: 3.0 },
            Instruction::LoadF { dst: Reg(1), value: 5.0 },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::Fdim.to_byte(),
                operands: vec![2, 0, 1],
            },
            Instruction::Ret { value: Reg(2) },
        ]);

        let module = create_test_module(bytecode, 4);
        let mut state = InterpreterState::new(module);
        state.call_stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        state.registers.push_frame(4);

        let result = dispatch_loop_table(&mut state).unwrap();
        assert!((result.as_f64() - 0.0).abs() < 1e-10);
    }
}
