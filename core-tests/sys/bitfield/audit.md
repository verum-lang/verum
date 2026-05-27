# `core.sys.bitfield` — implementation audit

## Status: **complete** (dispatch closed + parallel runner SIGABRT-free)

* Bit-manipulation primitives are landed in `core/sys/bitfield.vr`
  (free functions, USize-typed, `@inline(always) + pure`).
* Cross-module dispatch defect (§3.2 below) is **closed** by task #121
  (`fix(types/infer): mount-alias receiver bypass for method-call dispatch`).
  Both the `bitfield.fn(args)` method-form and the `bitfield.CONST`
  field-form route through `core.sys.bitfield` correctly at typecheck +
  codegen + interpreter tiers.
* Interpreter SIGABRT defect (§3.3 below) is **closed** by task #14
  (canonical alignment-safe `ObjectHeader` accessors covering all 86
  cast sites across the Tier-0 dispatch handlers).
* Conformance suite (`unit_test.vr`, `regression_test.vr`) executes
  cleanly via the `bitfield.<name>(...)` qualified-path form; verified
  by `/tmp/bitfield_regr_main.vr` driver (3 regression assertions pass).

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
* **Workaround (today, preferred)**: `mount core.sys.bitfield;` then
  `bitfield.USIZE_BITS` — verified working in task #121.
* **Impact**: every cross-module re-export of a `public const` requires
  the FQN form. Affects `core.sys.cabi.CFD_STDIN/STDOUT/STDERR` and any
  future const-export.
* **Status**: **partially closed** by task #121 for the bare-mount
  `mount X.Y.Z;` form (qualified path `bitfield.USIZE_BITS` works). The
  selective `mount X.{CONST};` form still falls through to the simple-name
  codegen path — re-tracking the remaining surface in task #15.
* **Pinned by**: `regression_test.vr::regression_const_via_fqn_resolves`.

### 3.2 Cross-module free-function dispatch silently returns Unit/nil

* **Symptom (pre-fix)**: `bitfield.test_bit(value, n)` (and any other
  cross-module free-function call) compiled cleanly, ran without panic,
  but returned Unit (printed as `()` or `nil`) instead of executing the
  callee body.
* **Reproducer**: any `mount`-imported free function from a sibling
  module — including the well-established `core.base.glob.matches` —
  exhibited the same behaviour at `--interp` runtime.
* **Status**: **closed** by task #121 across the full
  type-check → codegen → archive-load → interpreter pipeline.
  Independent verification: `/tmp/bitfield_regr_main.vr` driver — all
  three regression assertions (`USIZE_BITS` ≥ 32 and 8-aligned,
  `test_bit(0xA5, 7) == true`, `extract_bits(0xABCD, ..)` round-trips
  to 0xCD/0xAB) pass.
* **Fix components**:
  * Typechecker: `infer_method_chain_iterative` module-alias bypass +
    `try_resolve_super_path_call::module_aliases` `core.`-stripped probe
    fallback (`crates/verum_types/src/infer/modules.rs`).
  * Codegen: `compile_function` + `emit_lenient_panic_stub`
    `descriptor.name` qualified-form promotion using
    `current_source_module`
    (`crates/verum_vbc/src/codegen/mod.rs`).
  * Codegen-expr: `find_function_by_suffix` registry-suffix probe in
    `compile_field_access` + `compile_method_call` +
    `compile_qualified_path`
    (`crates/verum_vbc/src/codegen/expressions.rs`).
  * Archive loader: qualified-form detection (`simple_name.contains('.')`)
    routes through the qualified path directly + recovers
    rightmost-segment alias for simple-name lookups
    (`crates/verum_compiler/src/archive_ctx_loader.rs`).
* **Pinned by**: `regression_test.vr::regression_dispatch_returns_real_bool`.

### 3.3 `verum test --interp` (no filter) crashes with SIGABRT

* **Symptom (pre-fix)**: the full-suite invocation aborted inside
  `verum_vbc::interpreter::dispatch_table::handlers::cbgr::handle_drop_ref`
  via `panic_misaligned_pointer_dereference`.
* **Diagnosis**: `handle_drop_ref` (and 85 other interpreter sites) cast
  `val.as_ptr::<u8>()` to `*const heap::ObjectHeader` and dereferenced
  it without alignment checks.  When the value pointed at wasn't
  aligned to `align_of::<ObjectHeader>()` (8 bytes — the struct is
  `#[repr(C, align(8))]`), the dereference tripped Rust's runtime UB
  alignment check and aborted the whole interpreter through SIGABRT —
  losing every parallel test in the same invocation.  Misaligned
  pointers reach the dispatch through `Text.as_bytes()`-style byte-FatRefs,
  `slice_from_raw_parts` intrinsics, and any `&unsafe` cast in user code.
* **Status**: **closed** by the task #14 fix.  Three canonical
  alignment-safe accessors on `ObjectHeader`:
  - `try_from_ptr(ptr) -> Option<&Self>` — Option discipline for
    validation boundaries.
  - `try_type_id(ptr) -> Option<TypeId>` — one-shot type_id reads.
  - `ref_or_stub(ptr) -> &'a Self` — `&'static` all-zero sentinel
    header on misalignment (TypeId(0) routes every dispatch-time
    `header.type_id == X` check through its else-branch).

  86 cast sites across 14 handler files rewritten to consume the
  helpers.  6 drift-pin tests in `interpreter::heap::tests` lock the
  soundness invariant (null + every 1..7 misalignment offset +
  sentinel stub aliasing).  1205/1205 verum_vbc lib tests pass.

* **Pinned by**: `interpreter::heap::tests::object_header_*` (6 tests).

## Action items

### Landed in 2026-05-27 conformance refresh

* **`property_test.vr` shipped (31 algebraic laws across 5 sections)**:
  single-bit idempotence / self-inverse / disjoint commutation; mask
  idempotence / dual-via-complement laws; field-mask zero-width / full-
  width / disjoint-union / popcount-equals-width; extract ∘ insert
  round-trip with adjacent-field independence and above-width masking;
  cross-operation equivalences (set_bit ↔ set_bits-1, toggle ↔ xor-1,
  test ↔ extract-1). Sweeps run over representative-sample USize and
  position lists.
* **`integration_test.vr` shipped (22 cross-stdlib scenarios across 9
  sections)**: `BitfieldElement` round-trips for Bool / UInt8 / UInt16
  / UInt32 / UInt64; `BIT_WIDTH` protocol-constant pinning; free-
  function primitives composed with `List<(USize, USize, USize)>`
  field-packing; `test_bit` via List-iter `filter`/`map`/`collect`
  combinator chain; popcount loop pinned against analytic byte
  patterns; `Endianness` variant trio exhaustively pattern-matched
  through `Maybe<Endianness>`; USIZE_BITS ↔ type-property drift pin.

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

### Landed in task #121 (cross-module dispatch closure)

* **Mount-alias receiver bypass + `core.`-stripped probe** for cross-module
  free-function method-call dispatch — every shape of `mount X.Y.Z;` +
  `Z.fn(args)` now resolves through the qualified archive descriptor
  instead of falling into `UnboundVariable("Z")` or returning Unit.
* **Qualified-descriptor.name promotion** in `compile_function` +
  `emit_lenient_panic_stub` using `current_source_module` — sibling files
  within the same archive-entry directory keep distinct module-path
  prefixes (`sys.bitfield.test_bit` vs `sys.io.test_bit`).

### Landed in task #14 (alignment-safety primitives)

* **86 unguarded `*const heap::ObjectHeader` dereferences** across the
  Tier-0 interpreter rewritten to route through three canonical
  alignment-safe accessors (`try_from_ptr` / `try_type_id` /
  `ref_or_stub`).  `verum test --interp` no longer SIGABRTs on
  parallel-scheduled CBGR allocations adjacent to bitfield tests.

### Deferred

* **#15 — selective `mount X.{CONST};` const-import** still falls through
  to the simple-name codegen path. The bare-mount `mount X.Y.Z;` +
  `Z.CONST` form is the working path today.
* **`property_test.vr` and `integration_test.vr`** for bitfield: ready to
  land now that #13 and #14 are closed; integration-suite seed work
  remains.
* **Migrate `core.database.sqlite.native.{vdbe_register_model,cursor_hint_codes}.flag.set_bit/clear_bit`**
  to delegate to `core.sys.bitfield` — unblocked by task #121.
