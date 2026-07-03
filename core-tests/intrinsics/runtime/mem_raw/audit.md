# `intrinsics/runtime/mem_raw` audit

Module: `core/intrinsics/runtime/mem_raw.vr` (~175 LOC) — raw-address
(Int-pointer) memory operations: memcpy / memmove / memset / memzero /
memcmp, C-string strlen / strcmp, byte/word load-store.  Migrated from
`core/mem/raw_ops.vr` (#64 Phase 1.4).

Tests: unit (18) + property (10) + integration (6) + regression (1), all
driving LIVE allocations from the `cbgr_allocate` bridge (see the cbgr
audit) — this closes the MEM-RAWPTR-HARNESS-1 "needs a real allocation"
gap without depending on `List.as_mut_ptr` (LIST-ASPTR-HEADER-1).

## 1. Defects FIXED on this branch (2026-07-03)

### MEMRAW-CANONICAL-NAMES-INERT-1 — the whole byte/word surface was inert under the interpreter

The interpreter's name-dispatch table (`handlers/calls.rs`) matched only
the legacy underscore aliases (`__load_byte`, `__store_byte`, `__load_i64`,
`__store_i64`, `__load_i32`, `__store_i32`).  The #64 migration renamed the
canonical intrinsic keys to the bare forms — which fell through to the .vr
stub bodies (`{ 0 }`): **loads returned 0, stores did nothing**, while AOT
lowered to real machine ops.  Every fallback body in the module
(memcpy/memmove/memset/memcmp/strlen/strcmp route through
`__load_byte`/`__store_byte`) was equally inert, so memcmp compared
0-to-0 ("equal"), memset "succeeded" invisibly, and only tests that
never read memory back appeared green.  **Fix**: both spellings dispatch
to the real handlers (`"__load_byte" | "load_byte" => …`).

This is a textbook instance of the intrinsic-dispatch-contract drift class
(body `@intrinsic` vs table authority): the stub body silently shadowed the
missing table entry.

## 2. Open questions / deferred

* **memcpy/memmove/memset/memcmp dispatch route**: these have real handlers
  (name-dispatched) AND interpreter fallback bodies.  The conformance laws
  (copy⇒equality, overlap shifts both directions, first-NUL strlen) now pin
  the SEMANTICS regardless of route; the dispatch-contract doc pins the
  authority.  If a future profile shows the .vr fallback bodies executing,
  route them to `CMemcpy`/`CMemset`/`CMemmove`/`CMemcmp` sub-ops (0x43-0x46).
* **store_i32 sign-extension on load**: `load_i32` reads an i32; the
  contract pinned here is low-32-bit preservation.  Whether the top half is
  sign- or zero-extended is representation business — pin it once both
  tiers agree on an explicit rule (candidate: sign-extend, matching the
  `i32` C type).
* **Endianness**: pinned against `platform.target_is_little_endian()` in
  integration; a big-endian target would exercise the other branch.

## 3. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.*` | bulk copies during grow/realloc paths. |
| `core.sys` / FFI | C-string interop (strlen/strcmp over foreign buffers). |
| `core.text` | byte-buffer manipulation on the no-alloc path. |

## 4. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs` —
  the byte/word name-dispatch arms (both spellings).
* `crates/verum_codegen/src/llvm/` — AOT lowering of
  memcpy→`llvm.memcpy.p0.p0.i64` etc.
* `core/intrinsics/runtime/mem_raw.vr` fallback bodies — must stay
  semantically identical to the table handlers (the property laws are the
  tripwire).

## 5. Action items

**Landed this branch**
* MEMRAW-CANONICAL-NAMES-INERT-1 (above).
* Full allocation-backed conformance suite (unit/property/integration/
  regression) — byte-granularity pinned (#38 class), overlap laws, C-string
  laws, platform-endianness cross-check.

**Deferred (tracked)**
* Route fallback bodies to the 0x43-0x46 sub-ops if profiling shows them hot.
* Sign-extension rule for `load_i32` (cross-tier pin).
