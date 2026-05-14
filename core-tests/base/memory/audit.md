# `core.base.memory` — audit findings

> Module under test: `core/base/memory.vr` (1779 LOC after dedup of duplicate
> `Heap.is_freed` definition; 88 public surfaces across `Heap<T>`,
> `Shared<T>`, `Weak<T>`, `Cow<T>`, `Pin<T>`, `ManuallyDrop<T>`,
> `MaybeUninit<T>`, plus `drop` / `is_null` / `zero` / `swap` / `replace` /
> `take` free functions and the re-exported memory intrinsics from
> `core.intrinsics.memory.{ptr_read, ptr_write, ptr_offset, drop_in_place,
> forget, ptr_is_null, null_ptr, slice_from_raw_parts,
> slice_from_raw_parts_mut, memcpy, memmove, memset, memcmp, uninit, zeroed}`.)
>
> Test surfaces: `unit_test.vr` (131 LOC), `property_test.vr` (172 LOC),
> `integration_test.vr` (102 LOC), `cbgr_test.vr` (928 LOC, migrated from
> the bootstrap VBC interpreter conformance set), `regression_test.vr` (new
> in this branch).

## 1. Cross-stdlib usage

`Heap<T>` and `Shared<T>` are foundational — every other stdlib module that
needs heap allocation either constructs them directly or piggybacks on
their CBGR generation tracking. Notable consumer sites:

| Consumer | Use |
|---|---|
| `core/collections/list.vr` | `List<T>` payload pointer is allocated via the same CBGR allocator surface (`cbgr_alloc` / `cbgr_dealloc`); `List` does not wrap `Heap<T>` directly. |
| `core/async/future/*` | `Heap<dyn Future>` boxing of erased future bodies. |
| `core/io/buf_reader.vr` | `Shared<File>` for fan-out without re-opening. |
| `core/net/weft/bufpool.vr` | mounts `core.base.memory.{ptr_offset}` directly. |
| `core/cell.vr` | `RefCell<T>` body uses `MaybeUninit<T>` for the un-initialised seed slot. |

The re-export shape (`public mount core.intrinsics.memory.{...}` at lines
74-90) is structurally important: external callers can mount
`core.base.memory.{ptr_offset, drop_in_place, forget, ptr_is_null}` and
get the inline-emitting intrinsics — not a wrapper — so call sites have
zero overhead at every tier. The historical bug at lines 58-73 (bare-name
`ptr_read` wrapper colliding with the pre-registered intrinsic via
`compile_call`'s leaf-fallback at `expressions.rs:4116`) is documented
inline in the source; do not re-add `fn ptr_*` wrappers in this file.

## 2. Crate-side hardcodes

The compiler's `verum_common::well_known_types` has a number of pinned
TypeId / canonical-name constants that this module's runtime depends on.
Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `WKT_HEAP` (verum_types/infer/mod.rs) | `Heap` canonical name | Method-dispatch fallback for `Heap<T>` route would mis-resolve. |
| `WKT_SHARED` (verum_types/infer/mod.rs) | `Shared` canonical name | Same; `.clone()` on `Shared<T>` could route to the wrong impl. |
| `TypeId::HEAP` (verum_vbc/well_known_typeids.rs) | Heap heap-header TypeId | NaN-boxed pointer dispatch would mis-classify. |
| 32-byte CBGR header offset | `header_addr = self.ptr as Int - 32` (4 sites in memory.vr lines 263/372/383/406/415/443) | If the allocator header grows past 32 bytes, every `is_valid`/`is_allocated`/`is_freed`/`current_epoch`/`capabilities` validation reads garbage. Pin this with a compile-time-asserted constant in the allocator. |
| `cbgr_alloc` return shape `(ptr, gen, epoch)` | 3-tuple — Heap/Shared/MaybeUninit unpack identically | If the allocator changes its return-shape these three call sites diverge silently. |
| `ORDERING_RELAXED/ACQUIRE/RELEASE/ACQ_REL/SEQ_CST` constants | Memory-ordering ints | LLVM lowering must agree on these magic numbers; cross-tier divergence is a kernel-soundness incident. |
| `intrinsic_name = "ptr_read"` propagation | `compile_function` must copy `intrinsic_name` to the descriptor (see MEMORY.md task #44) | If propagation regresses, `Heap.new(value)`'s `ptr_write` falls through to a body-recursion StackOverflow. |

## 3. Language-implementation gaps

### 3.1 Duplicate method definitions in impl blocks (LANDED in this branch)

Witnessed: `core/base/memory.vr` shipped with `Heap<T>.is_freed` defined
**twice**, identical bodies, lines 404-408 and 435-439. The
HashMap-backed `inherent_methods` registry at
`verum_types/infer/decls.rs:5246` silently overwrote on the second
`methods.insert()` — there was no diagnostic, no warning. A typo-named
duplicate would have shipped a wrong second body unnoticed.

**Fundamental fix** landed in this branch:
`TypeChecker::check_no_duplicate_impl_items` (new helper in
`verum_types/infer/decls.rs`) walks `impl_decl.items` once before
registration, building a `HashSet<String>` keyed by item name. Methods,
associated types, associated consts, and proof clauses share one namespace
inside an impl block (each can collide with each other under the
runtime's qualified-name dispatch). The check is called from both
`register_inherent_impl_methods` and `register_protocol_impl_methods`,
emitting `TypeError::Other` with a clear "duplicate {kind} '{name}' in
impl block" diagnostic.

**Stdlib companion fix**: the second duplicate `is_freed` body was
removed from `core/base/memory.vr` in the same commit.

**Pinned by**: `regression_test.vr §A`.

### 3.2 CBGR header offset is a literal `32` repeated across 6 sites

The CBGR allocation header sits at `ptr - 32` and is currently expressed
as the integer literal `32` at six different sites inside
`core/base/memory.vr`. If the allocator's `AllocationHeader` ever changes
size (e.g. extending to 40 bytes for added capability bits), every one
of these will read garbage and every `is_valid` / `is_allocated` /
`is_freed` / `current_epoch` / `header_epoch` / `capabilities` /
`header_size` will start lying.

**Action item (deferred — separate task)**: replace each `- 32` with a
`Heap.header_size_for_validation()` const or import
`core.mem.header.AllocationHeader.SIZE`. The const must agree with the
Rust-side `verum_runtime::cbgr::HEADER_SIZE` — pin via a drift unit test
in `crates/verum_common/src/well_known_types.rs` (mirroring the
`primitive_protocol_matrix_pinned` pattern).

### 3.3 `Heap.new` infinite-recursion class (CLOSED — task #44)

Previously surfaced as `StackOverflow inside MaybeUninit.write / Heap.new`
when bare-name `ptr_read` resolved to the LOCAL wrapper instead of the
archive-loaded intrinsic. Closed by commit `698795e39` (descriptor
intrinsic_name propagation in `compile_function`). Pinned by interpreter
trace gating on `VERUM_TRACE_PUSH_STR=1` instrumentation in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs:429`.

### 3.4 14 of 18 `cbgr_test` heap tests failing in interpreter (OPEN)

Baseline interpreter run (`verum test --interp --filter cbgr_test::test_heap_`,
263s total): 4 passed, 14 failed. Two captured failure shapes both report
`AssertionFailed { message: "assertion failed: left != right", pc: N }`
— `test_heap_default` (pc=25) and `test_heap_cbgr_after_modification`
(pc=88). The remaining 12 failures' assertion text was not echoed by the
terse-format runner — re-run with `--nocapture` is the next diagnostic
step.

Hypothesis: `Heap.new_default()` (line 180) goes through
`T.default()` → primitive-protocol-default dispatch which currently
resolves to a non-zero sentinel for `Int.default()` in the
interpreter's `default()` fallback path. Verify by running
`cbgr_test::test_heap_default` with `--interp --nocapture` and inspecting
the asserted vs actual value at pc=25.

**Action items deferred**:

  - **§A** Run `cbgr_test::test_heap_*` with `--nocapture` to capture
    each of the 14 assertion messages; categorise the failure shapes.
  - **§B** Cross-check whether the same 14 fail in AOT — if AOT-only or
    interp-only, that's a tier-divergence kernel incident.
  - **§C** Build a per-test status appendix below once §A completes.

### 3.5 AOT lowering fails on `Heap.new(_).is_freed()` (CROSS-TIER divergence)

Runtime-validated: `verum test --aot --filter regression_heap_round_trip_baseline`
fails at LLVM lowering with

```
compilation failed: Failed to lower VBC to LLVM IR:
Internal("call_native_i64(lr): callee returns void; use build_call directly")
```

while the same source passes under `--interp`. The error is raised by
`crates/verum_codegen/src/llvm/platform_ir.rs:246` —
`call_native_i64` expects every native FFI callee to return an
IntType / PointerType, but somewhere along the `Heap.new` →
`cbgr_alloc` → allocator path a callee is declared with a `void`
return type, then call_native_i64 is reached for it and refuses.

This is the SAME failure class as MEMORY.md task #23's residual entry:

> AOT proceeds, falls back to interpreter on a different remaining
> error (`call_native_i64(lr): callee returns void; use build_call
> directly`)

So this defect surfaces on the most basic `Heap.new` allocation, not
on a niche path. Fix layer: the registration site that declares
`verum_internal_close` / `verum_internal_*` wrappers with the wrong
return type must reconcile with the `i64`-ABI invariant declared by
`crates/verum_codegen/src/llvm/runtime.rs:8271+ get_or_declare_close`.
The adopt-and-emit path there explicitly states the wrapper "returns
i64" — so the divergence is upstream: an earlier code path is
forward-declaring the wrapper with a void return before
`get_or_declare_close` runs, and the i64-promotion never overwrites
the prior signature.

**Pinned by**: `regression_test.vr §B` (round-trip baseline) — pin
fails in AOT, passes in interp; the CROSS-TIER DIVERGENCE is itself
the regression.

### 3.6 `Heap.is_freed` returns Bool but the post-dealloc generation may not always be 0

`is_freed` (line 404) checks `actual_gen != self.generation`. After a
properly-tracked `cbgr_dealloc`, the header's generation is incremented
(not zeroed), so any reused slot can return a *different non-zero*
generation. Functional, but the docstring at line 402 says "the
generation in the heap header has been incremented past the stored
generation value" — `actual_gen != self.generation` is the bidirectional
inequality, while a *strict* "incremented past" would be
`actual_gen > self.generation` (with wrap-around handling). Pinned as
**§D** for a future audit cycle.

## Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Duplicate `Heap.is_freed` definition (stdlib) | `core/base/memory.vr` lines 430-439 | Remove second definition; body was identical to the first at lines 404-408. |
| 2 | Compiler silently accepts duplicate-name methods in impl block (language) | `crates/verum_types/src/infer/decls.rs` | New `check_no_duplicate_impl_items` helper called from `register_inherent_impl_methods` and `register_protocol_impl_methods`; emits `TypeError::Other` with a "duplicate {kind} '{name}' in impl block" diagnostic before the silent-overwrite path can fire. |
| 3 | Missing `regression_test.vr` for `core-tests/base/memory/` | `core-tests/base/memory/regression_test.vr` | New file with §A duplicate-method post-fix shape, §B-D `Heap.new` / nested / `into_inner` round-trip pins. |
| 4 | Missing `audit.md` for `core-tests/base/memory/` | `core-tests/base/memory/audit.md` | This file. |
| 5 | Missing `docs/stdlib/memory.md` reference page | `internal/website/docs/stdlib/memory.md` | New page with the full public surface, status badge, and method-dispatch surface table. |

## Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Capture 14 `cbgr_test::test_heap_*` interpreter failures with `--nocapture` | ~10 min | open |
| §B | Cross-check AOT status of the same 14 | ~15 min (each AOT test ~30s) | blocked on §A |
| §C | Per-test status appendix below this section | ~5 min after §A,§B | blocked on §A |
| §D | `Heap.is_freed` strict-greater-than semantics audit + wrap-around handling | ~30 min | open |
| §E | Hoist `- 32` literal CBGR header offset into a single const (see §3.2) | ~20 min | open |
| §F | Fix AOT `call_native_i64(lr): callee returns void` on `Heap.new` (see §3.5) — reconcile the forward-declare path with `get_or_declare_close`'s i64 ABI in `crates/verum_codegen/src/llvm/runtime.rs:8271+` | ~2 h (kernel-level) | open |
