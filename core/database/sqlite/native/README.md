# core/database/sqlite/native/ — loom

Pure-Verum SQLite reimplementation. Spec: `internal/specs/sqlite-native.md`.

## Status (2026-04-21, v0.1 scaffolding)

| Layer | Module | Status | LOC |
|---|---|---|---|
| L0 | `l0_vfs/vfs_protocol.vr` | Scaffolded — protocol defined | ~60 |
| L0 | `l0_vfs/memdb_vfs.vr` | Minimal in-memory backing | ~140 |
| L0 | `l0_vfs/posix_vfs.vr` | NOT STARTED — needs Gap 1/2/3 | 0 |
| L1 | `l1_pager/` | NOT STARTED | 0 |
| L2 | `l2_record/serial_type.vr` | SerialType + record-header parse | ~130 |
| L2 | `l2_record/affinity.vr` | NOT STARTED | 0 |
| L3 | `l3_btree/` | NOT STARTED | 0 |
| L4 | `l4_vdbe/` | NOT STARTED | 0 |
| L5 | `l5_sql/` | NOT STARTED | 0 |
| L6 | `l6_session/` | NOT STARTED | 0 |
| L7 | `l7_api/` | NOT STARTED | 0 |

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

Documented for future root-cause fixes in the compiler/runtime crates:

### 1. `[lenient] SKIP` silently drops methods in `implement` blocks

Location: `crates/verum_vbc/src/codegen/mod.rs:2363`

When `compile_function` for an `implement Type { fn ... }` method fails,
it is silently dropped with only a `tracing::debug!` message, leading to
runtime `method '...' not found on value` panics rather than compile-time
diagnostics. Reproduced when the `MemDbFile` impl block was declared in a
new cross-module directory (`core/database/sqlite/native/l0_vfs/`).

**Suggested fix:** surface `compile_function` errors as hard compile-
time diagnostics, or at minimum emit a `tracing::warn!` visible at
default verbosity so users notice methods vanishing.

### 2. UInt32 XOR sign-extended when table lookup bit 31 = 1

Location: `crates/verum_vbc/src/interpreter/`

Example:
```verum
let table: [UInt32; 256] = [...];
let s: UInt32 = table[idx];   // if t[idx] bit 31 = 1, s now stored as i64
// with upper 32 = 0xFFFFFFFF; subsequent `s >> 8` carries those bits down.
```

Workaround currently shipped in `core/security/hash/crc32.vr` — an explicit
`& 0xFFFFFFFF_u32` after every XOR with a table entry to mask off the
sign-extended upper half. A true fix should keep `[UInt32; N]` reads
zero-extended through the NaN-boxed value representation.

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
