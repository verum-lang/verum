//! Handler modules for VBC interpreter dispatch.
//!
//! These modules contain extracted handler functions from dispatch_table.rs,
//! organized by category for maintainability.

// Shared helpers (used by multiple handler modules)
pub(super) mod bytecode_io;
pub(super) mod arith_helpers;
pub(super) mod string_helpers;
pub(super) mod cbgr_helpers;

// Data movement and type conversions (0x00-0x0F)
pub(super) mod data_movement;

// Arithmetic (0x10-0x2F)
pub(super) mod integer_arith;
pub(super) mod float_arith;

// Bitwise + generic arithmetic (0x30-0x3F)
pub(super) mod bitwise;

// Comparison (0x40-0x4F)
pub(super) mod comparison;

// Control flow + logic (0x50-0x5F subset: jumps, returns, logic)
pub(super) mod control_flow;

// Call operations (0x5B-0x5F, 0x80-0x82, 0x8A)
pub(super) mod calls;

// Debug, assert, panic (0xD6-0xD9)
pub(super) mod debug;

// Memory + Collections (0x60-0x6F, 0xC7-0xCF, 0xDD)
pub(super) mod memory_collections;

// CBGR references (0x70-0x78)
pub(super) mod cbgr;

// Pattern matching + variants (0x80-0x95 subset)
pub(super) mod pattern_matching;

// Iterators + ranges (0xC0-0xC1, 0xCC)
pub(super) mod iterators;

// String operations (0xC5-0xC6, 0xCB)
pub(super) mod string_ops;

// Generators (0xC2-0xC4)
pub(super) mod generators;

// Exception handling (0xD0-0xD3)
pub(super) mod exceptions;

// Method dispatch (CallM 0x5D resolution)
pub(super) mod method_dispatch;

// Async + Nursery operations (0xA0-0xAD)
pub(super) mod async_nursery;

// Context system + capabilities (0xB0-0xB7)
pub(super) mod context;

// System operations (0xE0-0xEF: syscall, atomic, TLS, mmap, IO, autodiff)
pub(super) mod system;

// Meta operations (0xB8-0xBB)
pub(super) mod meta;

// Extended opcode handlers
pub(super) mod text_extended;
pub(super) mod ffi_extended;
pub(super) mod math_extended;
pub(super) mod simd_extended;
pub(super) mod char_extended;
pub(super) mod log_extended;
pub(super) mod arith_extended;
pub(super) mod tensor;
pub(super) mod gpu;
pub(super) mod tensor_extended;
pub(super) mod ml_extended;
