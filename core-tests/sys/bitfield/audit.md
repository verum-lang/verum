# `core.sys.bitfield` — implementation audit

## Status: **partial** (regression-only on dispatch)

* Bit-manipulation primitives are landed in `core/sys/bitfield.vr`
  (free functions, USize-typed, `@inline(always) + pure`).
* Conformance suite is **regression-only** today: the unit tests live in
  `unit_test.vr` but every call site is gated behind the
  cross-module dispatch defect tracked in §3.2. As soon as the compiler
  closes that defect, the suite turns green without any source change at
  the test side.

## 1. Cross-stdlib usage

`extract_bits` / `insert_bits` / `test_bit` / `set_bit` / `clear_bit` /
`toggle_bit` previously had a **parallel UInt64-typed implementation** in
`core/math/bits.vr` (lines 277-310 in the pre-fix file). The two parallel
free-function definitions registered the same names twice in the global
symbol table — under monomorphisation the codegen picked one
non-deterministically, surfacing as silent miscompilations on USize call
sites.

**Action landed**: deleted the UInt64-typed bit-field block from
`core.math.bits`; the module's docstring now points at
`core.sys.bitfield` for these primitives. `core.math.bits` retains all
the genuinely math-domain operations (`clz`, `ctz`, `popcnt`, `bswap`,
`bitreverse`, rotation, Morton interleaving) which have no overlap.

Other consumers in the workspace that touch the same names but stay
within their own type domain (so no cross-module ambiguity):

| File | Function | Type | Note |
|---|---|---|---|
| `core/sys/mmio.vr:262` | `Register::set_bits` (method) | `&self, UInt32` | MMIO register mutator — unrelated to free-function `set_bits` |
| `core/sys/mmio.vr:269` | `Register::clear_bits` (method) | `&self, UInt32` | same |
| `core/sys/mmio.vr:276` | `Register::toggle_bits` (method) | `&self, UInt32` | same |
| `core/collections/bloom.vr:263, 269` | `BloomFilter::set_bit / test_bit` (methods) | `&self, Int` | Bloom-filter bit storage |
| `core/net/tls13/handshake/zero_rtt_antireplay.vr:181, 187` | `set_bit / test_bit` (free) | `&Bucket, Int` | 0-RTT replay bucket; takes `&Bucket`, no overlap with USize-typed primitives |
| `core/database/sqlite/native/vdbe_register_model/flags.vr:31, 35` | `set_bit / clear_bit` (free) | `Int64, Int64` | SQLite VDBE flags; the Int64 typing means the codegen *should* disambiguate, but task #13 tracks the same dispatch gap that surfaces here |
| `core/database/sqlite/native/cursor_hint_codes/flag.vr:22, 26` | `set_bit / clear_bit` (free) | `Int64, Int64` | Same shape as VDBE flags |

The two SQLite consumers are the next refactor candidates: they should
delegate to `core.sys.bitfield` once the cross-module dispatch defect is
closed, removing two more parallel free-function declarations.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_ast/src/bitfield.rs:474` | `BitfieldDef::field_mask(index)` returns the mask of a *specific layout entry*; this is the AST-side helper that the code generator emits the same expression for — independent of the runtime free function `core.sys.bitfield.field_mask(offset, width)` | OK; no naming collision |

No other Rust-side hardcodes for the new free functions surfaced.

## 3. Language-implementation gaps surfaced by this suite

### 3.1 `mount X.{public_const}` does not resolve cross-module constants

* **Symptom**: `mount core.sys.bitfield.{USIZE_BITS}` followed by reference
  to `USIZE_BITS` produces `UndefinedVariable("USIZE_BITS")` at codegen
  time.
* **Workaround**: `mount core.sys.bitfield;` then `bitfield.USIZE_BITS`.
* **Impact**: every cross-module re-export of a `public const` requires
  the FQN form. Affects `core.sys.cabi.CFD_STDIN/STDOUT/STDERR` and any
  future const-export.
* **Tracked in**: task **#15**.
* **Pinned by**: `regression_test.vr::regression_const_via_fqn_resolves`.

### 3.2 Cross-module free-function dispatch silently returns Unit/nil

* **Symptom**: `bitfield.test_bit(value, n)` (and any other cross-module
  free-function call) compiles cleanly, runs without panic, but returns
  Unit (printed as `()` or `nil`) instead of executing the callee body.
* **Reproducer**: any `mount`-imported free function from a sibling
  module — including the well-established `core.base.glob.matches` —
  exhibits the same behaviour at `--interp` runtime.
* **Workaround**: none from the test side. Every cross-module call needs
  the dispatch table fix.
* **Impact**: blocks the entire conformance suite for `core.sys.bitfield`
  and any other module whose tests rely on cross-module free-function
  calls.
* **Tracked in**: task **#13**.
* **Pinned by**: `regression_test.vr::regression_dispatch_returns_real_bool`.

### 3.3 `verum test --interp` (no filter) crashes with SIGABRT

* **Symptom**: the full-suite invocation aborts inside
  `verum_vbc::interpreter::dispatch_table::handlers::cbgr::handle_drop_ref`
  via `panic_misaligned_pointer_dereference`. Crash report at
  `~/.verum/crashes/verum-2026-05-10T19-55-53-...log`.
* **Diagnosis**: `handle_drop_ref` casts `val.as_ptr::<u8>()` to
  `*const heap::ObjectHeader` and dereferences `(*header).type_id` (line
  513-516 of `crates/verum_vbc/src/interpreter/dispatch_table/handlers/cbgr.rs`).
  When the value pointed at is not aligned to
  `align_of::<ObjectHeader>()` the dereference traps at the
  Rust-runtime level. Independent of this suite — pre-existing — but
  surfaces every time the parallel test runner happens to schedule a
  CBGR-allocated object next to the bitfield tests.
* **Tracked in**: task **#14**.

## Action items

### Landed in this branch

* **Bit-manipulation primitives canonicalised in `core.sys.bitfield`**:
  8 free functions (`test_bit`, `set_bit`, `clear_bit`, `toggle_bit`,
  `set_bits`, `clear_bits`, `extract_bits`, `insert_bits`) plus
  `field_mask` builder and `USIZE_BITS` constant.
* **Boundary-correctness contract documented** in module header (the
  `width == 0` / `width >= USIZE_BITS` / hot-path table) — the
  branchless hot path matches LLVM's defined-behaviour shift envelope.
* **Parallel UInt64 implementations removed** from `core.math.bits`;
  module-level docstring redirects readers to the canonical home.
* **`USIZE_BITS` and `field_mask` exported** from `core.sys.mod.vr`'s
  `bitfield.{...}` re-export list.
* **Conformance suite seeded**: `unit_test.vr` (12 sections, 56
  tests) + `regression_test.vr` (3 pinned defects) + this `audit.md`.
* **Three compiler-side defects identified, reproduced, and tracked**:
  tasks #13, #14, #15.

### Deferred

* **#13 / #15 — cross-module dispatch + const-import fixes**: required
  for the conformance suite to actually pass at runtime. Both are
  codegen + dispatch-table changes in `verum_vbc`; out of scope for
  the bitfield-implementation task and tracked separately.
* **#14 — `handle_drop_ref` alignment fix**: required for the
  parallel test runner to complete a full-suite run without aborting.
  Independent of bitfield testing; tracked separately.
* **`property_test.vr` and `integration_test.vr`** for bitfield: deferred
  until the cross-module dispatch defect (#13) is closed — until then,
  every property test asserts on a function that returns Unit.
* **Migrate `core.database.sqlite.native.{vdbe_register_model,cursor_hint_codes}.flag.set_bit/clear_bit`**
  to delegate to `core.sys.bitfield` once #13 is closed.
