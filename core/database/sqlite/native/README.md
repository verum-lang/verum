# core/database/sqlite/native/ — loom

Pure-Verum SQLite reimplementation: file format, pager/WAL journaling, btree, VDBE and session layers, built up as l0_vfs → l6_session (see the layer directories below).

## Status (measured 2026-07-30)

Code volume per layer, measured by `find <dir> -name '*.vr' | wc` — LOC is
NOT a completeness claim. The per-layer completeness audit against the spec
is pool task T0469; wiring status of what exists is tracked by the LOOM
stage tasks (T0537 scalar dispatch, T0538 name collisions, T0539 re-point
onto production stdlib, T0555 e2e reds).

| Layer | Module | Files | LOC |
|---|---|---|---|
| L0 | `l0_vfs/` | 9 | 3,035 |
| L1 | `l1_pager/` | 22 | 7,236 |
| L2 | `l2_record/` | 7 | 1,371 |
| L3 | `l3_btree/` | 13 | 4,183 |
| L4 | `l4_vdbe/` | 26 | 8,197 |
| L5 | `l5_sql/` | 18 | 11,828 |
| L6 | `l6_session/` | 9 | 2,880 |
| L7 | `l7_api/` | 10 | 3,058 |

Whole `native/` tree: 1,434 `.vr` files, ~138.7k LOC — much of it decision-table
and API-catalogue directories that are written but NOT wired into the live
engine (the wiring is exactly the LOOM stage tasks above). Known live-vs-written
splits: VDBE scalar dispatch uses 21 inline kinds while 118 implemented builtins
sit unwired (T0537); 75 `sqlite3_*` C-API dirs and 82 SQL parse/plan rule dirs
are unwired catalogues (folded into T0469's umbrella journal).

## Stdlib gaps closed before this work

| Gap | Module | Status |
|---|---|---|
| 4. Varint encoding | `core/encoding/varint.vr` | ✅ Production (VCS: L2/database/sqlite/l2_record/varint_roundtrip.vr) |
| 5. CRC32 / IEEE 802.3 | `core/security/hash/crc32.vr` | ✅ Production (VCS: crc32_vectors.vr) |
| 1. Byte-range locking | `core/sys/locking/` | ⛳ Pending |
| 2. Directory fsync | `core/sys/durability.vr` | ⛳ Pending |
| 3. Linux pread/pwrite | `core/sys/linux/io.vr` | ⛳ Pending |

## Tests (runnable in interpreter today)

| Test | Tier | Status |
|---|---|---|
| `vcs/.../l2_record/varint_roundtrip.vr` | 0 | ✅ passes (11 sub-tests) |
| `vcs/.../l2_record/crc32_vectors.vr` | 0 | ✅ passes (8 sub-tests) |

## Language-implementation issues surfaced during Phase 0 scaffolding

### 1. [FIXED — landed in binary] `[lenient] SKIP` silently drops methods

**Root cause:** the common case was *not* a genuine compile failure but a
disambiguation miss. In `compile_record` (plain struct / record-variant
literal), each field value expression was compiled without the enclosing
field's declared type in scope, so `find_function_by_suffix(".Variant")`
returned `None` whenever the same variant name existed in more than one
type (e.g., `Closed` in `DoorState` *and* `LockState`). The enclosing
method's codegen then bailed with `undefined variable: Closed`, the
`[lenient] SKIP` fallback dropped the method, and users got
`method '...' not found on value` at runtime.

**Fix** (`crates/verum_vbc/src/codegen/expressions.rs`):
`compile_record` now pushes the field's declared type into
`ctx.current_return_type_name` before compiling the field-value
expression and restores it afterwards, giving
`find_function_by_suffix` a disambiguation hint. New helper
`push_field_type_context` / `pop_field_type_context`. ~8 SKIP entries
in the stdlib (including `MemDbFile.size`) eliminated on first pass.

**Regression test:** `vcs/specs/L1-core/types/field_type_disambiguation.vr`
exercises two enums that share a variant name and verifies the method
containing a struct literal with that variant dispatches correctly.

### 2. [FIXED — source landed, binary awaits LLVM-env rebuild] UInt32 sign-extension on typed-array read

**Root cause (two places):**

1. `handle_get_index` in
   `crates/verum_vbc/src/interpreter/dispatch_table/handlers/memory_collections.rs`
   read 4-byte typed-array elements as `*const i32`, then cast to `i64`.
   Any element with bit 31 set (e.g., a CRC32 table entry like
   `0xB0D09822`) came back as `0xFFFFFFFF_B0D09822` because `i32 as i64`
   sign-extends. The fix reads as `*const u32` — zero-extension — and
   covers the `elem_size=2` path the same way (`u16` instead of `i16`).

2. `BinOp::Shr` in `crates/verum_vbc/src/codegen/expressions.rs` always
   emitted `BitwiseOp::Shr` (arithmetic shift). Even when the left
   operand is declared `UInt32`, the shift would sign-preserve. The fix
   inspects the left operand's inferred type name via
   `is_unsigned_int_type_name` and emits `BitwiseOp::Ushr` (logical)
   for `UInt8`/`UInt16`/`UInt32`/`UInt64`/`USize`/`Byte`, matching
   C and Rust semantics.

**Regression test:** `vcs/specs/L1-core/types/unsigned_shr_logical.vr`.

The `& 0xFFFFFFFF_u32` workaround in
`core/security/hash/crc32.vr::update` can be removed once the binary is
rebuilt — it is now a belt-and-suspenders clamp.

### 3. `Type.method()` style parsed as context-lookup

Example: `OpenFlags.default()` emits a
`Context 'OpenFlags' used but not declared in 'using' clause` warning
and then fails dispatch. Free-function pattern (`flags_default()`)
works correctly.

**Suggested investigation path:** `crates/verum_vbc/src/codegen/expressions.rs:~5000`
— the method-call compiler checks `required_contexts` before falling
through to static-method resolution. The check should be narrowed to
types actually declared in a `using [...]` clause.

### 4. Borrow-checker retains `&` past the originating scope

Example: after calling `free_fn(&file)` inside a `match` block and
assigning the result, the checker still treats `file` as immutably
borrowed at the next `&mut file` site even though no reference
escapes the match arm. NLL analysis needs tightening.

### 5. `assert!(…)` is parsed as a Rust-style macro invocation

The Verum parser emits a helpful error (`E0E2`), but the diagnostic
currently only fires at the top level of a statement; inside an
expression context (e.g., `assert!(x); assert!(y);`) the parser
recovery continues to flag the second invocation as a spurious error.

## Implementation notes

* `MemDbVfs` is fully self-contained — no OS calls — which makes it the
  DST baseline and lets L1/L2 development proceed before PosixVfs lands.
* Per sqlite-native.md §21, the B-tree and WAL layers will carry
  `@verify` contracts; SMT discharge happens in `verify/`.
* Any divergence from C-SQLite semantics is a bug — see `vcs/differential/sqlite/`.
