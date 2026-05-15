//! Handler modules for VBC interpreter dispatch.
//!

//! These modules contain extracted handler functions from dispatch_table.rs,
//! organized by category for maintainability.

// Shared helpers (used by multiple handler modules)
pub(super) mod arith_helpers;
pub(super) mod bytecode_io;
pub(super) mod cbgr_helpers;
pub(super) mod string_helpers;
// Shared heap-marshaling primitives for the Tier-0 intercept
// modules (shell/file/env/stdio/process/net). Single canonical
// source for `alloc_byte_list` / `alloc_record_n_fields` /
// `wrap_in_variant` / `extract_byte_slice` / `extract_text_arg` /
// `is_record_typed_as` / `lookup_type_id_by_name` etc.
pub(super) mod heap_helpers;
// `pub(crate)` rather than `pub(super)` so sibling interpreter
// modules (notably `interpreter::io_engine::async_accept`) can
// reach `net_runtime::NET_STATUS_*` constants for reactor
// dispatch without going through dispatch_table's private
// orchestration.
pub(crate) mod net_runtime;

// Data movement and type conversions (0x00-0x0F)
pub(super) mod data_movement;

// Arithmetic (0x10-0x2F)
pub(super) mod float_arith;
pub(super) mod integer_arith;

// Bitwise + generic arithmetic (0x30-0x3F)
pub(super) mod bitwise;

// Comparison (0x40-0x4F)
pub(super) mod comparison;

// Control flow + logic (0x50-0x5F subset: jumps, returns, logic)
pub(super) mod control_flow;

// Call operations (0x5B-0x5F, 0x80-0x82, 0x8A)
pub(super) mod calls;

// High-level Rust intercepts for shell-runtime calls (sh_check, sh).
// Bypasses the libSystem FFI chain — see VBC-1 architecture notes.
pub(super) mod shell_runtime;

// High-level Rust intercepts for file I/O (read_to_string, write,
// read, write_bytes, exists). Sibling to shell_runtime; same Tier-0
// architecture (bypass libSystem FFI, use std::fs directly).
pub(super) mod file_runtime;

// High-level Rust intercepts for env-var ops (var, var_opt, set_var,
// remove_var). Sibling to file_runtime; bypasses libSystem
// `getenv`/`setenv`/`unsetenv` via `std::env`.
pub(super) mod env_runtime;

// High-level Rust intercepts for stdin (read_line, read_int,
// read_float, read_to_end). Sibling to env_runtime; bypasses
// libSystem `read(2)` on stdin via `std::io::stdin()`.
pub(super) mod stdio_runtime;

// High-level Rust intercepts for Path/PathBuf inherent methods
// (as_path, to_path_buf, join, join_str, parent, as_str, to_str).
// Closes the gap left by stub-only registration: when codegen emits
// CallM for a stdlib method whose body is in the precompiled archive
// but not the user module, the synthesised RetV stub returns Unit
// and downstream record-field accesses crash.  These intercepts
// re-implement the body in Rust against the heap layout.
pub(super) mod path_ops_runtime;

// High-level Rust intercepts for `Text.*` static factory methods
// (`Text.new`, `Text.with_capacity`, `Text.from_static`,
// `Text.from_str`, `Text.from_char`, `Text.from_utf8`, …).
// Sibling to `path_ops_runtime` — closes the same architectural
// gap for the canonical stdlib-Text static surface.  When a user
// script writes `Text.with_capacity(64)` and codegen emits
// `Call { func_id: <archive_id> }`, the user module has no body
// for that id (the body lives in the precompiled archive); the
// synthesised RetV stub returns Unit and downstream
// `s.len()` / `s.capacity()` crash with `method not found on
// receiver of runtime kind ()`.  This intercept fires first and
// returns the canonical empty-or-built Text value.
pub(super) mod text_static_runtime;

// High-level Rust intercepts for `core.base.protocols.DefaultHasher`.
// Closes the precompiled-stdlib-body SetF-loss defect (task #11) by
// running the canonical FxHash 64-bit step directly on the heap
// record, bypassing the broken user-side body until the codegen-level
// fix lands.  See `hasher_runtime` doc comment for rationale.
pub(super) mod hasher_runtime;

// High-level Rust intercepts for `core.text.char.Char` `&mut self`
// mutators.  Closes the precompiled stdlib body's DerefMut-emit
// defect (task #14) — `*self = self.to_ascii_uppercase()` inside the
// `&mut self` method body emits Mov instead of DerefMut at precompile
// time, losing the mutation.  The intercept operates directly on the
// caller's slot via the CBGR-ref writeback discipline (same pattern
// as hasher_runtime).  See `char_runtime` doc comment for rationale.
pub(super) mod char_runtime;

// High-level Rust intercepts for V-LLSI context-system raw intrinsics
// (`__ctx_get_raw` / `__ctx_provide_raw` / `__ctx_end_raw` /
// `__defer_*_raw`).  The intrinsic registry has no entry for the
// `@intrinsic("ctx_get")` etc. annotations these declarations carry,
// so the codegen falls through and emits a single Return-opcode body —
// defeating the `bytecode_length == 0` gate in
// `try_dispatch_intrinsic_by_name`.  The intercept fires BEFORE the
// Call reaches the body and routes through the existing
// `state.context_stack` + `state.defer_stack` machinery.
pub(super) mod ctx_runtime;

// High-level Rust intercepts for process spawning
// (spawn_child_with_output for `Command.output()` / `.status()`).
// Sibling to shell_runtime; bypasses libSystem fork/execve/pipe via
// `std::process::Command`. See VBC-PROC-2 architecture notes.
pub(super) mod process_runtime;

// Runtime-bridge intercepts (#330): `verum_get_runtime_*` getters
// for manifest-driven runtime config. AOT emits LLVM-folded reads
// from `__verum_runtime_*` globals; under interpreter mode we
// return the documented `0` sentinel so AsyncRuntimeConfig.default()
// and friends work unchanged in `verum run`.
pub(super) mod runtime_bridge;

// Async-runtime intercepts (#334): `block_on` under interpreter
// mode where async fns are not compiled to suspend/resume state
// machines. The Verum-side `block_on` body calls `.poll()` on
// the value, but in interpreter mode the value IS the awaited
// result (not a Future). Intercept short-circuits to return
// the value directly. AOT keeps the full Future-poll dispatch.
pub(super) mod async_runtime;

// Backtrace.capture() intercept (#48): walk the VBC call stack and
// build a Verum Backtrace { frames: List<StackFrame> } value.
// Uses SourceMap entries for file/line/column when present.
pub(super) mod backtrace_runtime;

// catch_unwind(f) intercept (#50): run `f` via execute_table, catch
// InterpreterError::Panic and turn it into Result.Err(PanicInfo).
// Normal returns become Result.Ok(val). Non-panic errors propagate.
pub(super) mod panic_runtime;

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
pub(super) mod arith_extended;
pub(super) mod char_extended;
pub(super) mod cubical;
pub(super) mod extended;
pub(super) mod ffi_extended;
pub(super) mod gpu;
pub(super) mod log_extended;
pub(super) mod math_extended;
pub(super) mod ml_extended;
pub(super) mod simd_extended;
pub(super) mod tensor;
pub(super) mod tensor_extended;
pub(super) mod text_extended;
