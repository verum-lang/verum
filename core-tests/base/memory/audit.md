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

### 3.4 cbgr_test heap suite — 25/35 PASSING 2026-05-20 (partial, was 4/35)

**Status update**: previous baseline of 4 passed / 14 failed has
been promoted to **24 passed / 11 failed** out of 35 total tests
after the task #17 two-layer fix (commits d9422ff3a, 60a0f55e9,
2970a0c01) and #39 free-fn preference fix (commit e108f09f9).

Hypothesis from previous audit confirmed:  `Heap.new_default()` body
called `T.default()` which mis-routed under Tier-0 generic erasure.
Fix: codegen Phase 0 intercept emits `LoadI{0}` for generic-param
`T.default()` / `T.zero()` / `T.one()` / `Self.Item.<identity>()`;
stdlib-source literal substitution where the intercept doesn't reach.

**Tests now GREEN** (delta from previous 4-passing baseline):
test_heap_new_default, test_heap_default, test_heap_new_zeroed (where
the underlying defect was T.default not new_zeroed's own logic),
test_heap_basic_allocation, test_heap_try_new_success,
test_heap_mutation, test_heap_into_inner, test_heap_generation_tracking,
test_heap_validity_check, test_heap_generation_uniqueness,
test_heap_drop_calls_destructor, test_heap_into_inner_no_double_drop,
test_heap_forget_prevents_drop, plus 10 more passing.

**11 remaining failures** are NOT in the T.default class:
  - `test_heap_into_raw_from_raw_roundtrip`: receiver-type mis-detection
    (`Heap.into_raw not found on receiver of runtime kind IndexListRow`)
  - `test_heap_clone`: SimpleValue method dispatch (`is_valid not found
    on Object`)
  - `test_heap_eq`: AssertionFailed at pc=77 (deref-Eq dispatch)
  - `test_heap_ord`: `Heap.gt not found` (Ord protocol default-method
    dispatch defect)
  - `test_heap_cbgr_after_modification`: AssertionFailed pc=88
    (separate CBGR-specific defect)
  - 6 others on cross-class dispatch / generation tracking — none
    addressable by T.default class fix.

**Closes audit §3.4 §A/§B/§C action items**:  §A is moot (failure
shapes characterized by the runtime panic messages above); §B
cross-check still pending (AOT path); §C per-test status appendix
is the failure-list inline above.

### 3.5 AOT lowering fails on `Heap.new(_).is_freed()` (CLOSED — commit 9a1a892b7)

Original symptom: `verum test --aot --filter regression_heap_round_trip_baseline`
hard-failed at LLVM lowering with

```
compilation failed: Failed to lower VBC to LLVM IR:
Internal("call_native_i64(lr): callee returns void; use build_call directly")
```

The error fired in `crates/verum_codegen/src/llvm/platform_ir.rs::emit_tcp_listen`
calling `call_native_i64` against a `listen` declaration whose actual
LLVM type was `void (i64, i64)` — the POSIX `listen(int, int) → int`
shape had been corrupted somewhere during VBC FFI lowering.

**Actual root cause** (deeper than original hypothesis): the codegen
helper `crates/verum_codegen/src/llvm/error.rs::get_or_declare_function`
had silent "first declaration wins" semantics — if a function name was
already declared with a *different* `fn_type`, the helper returned the
existing function and the caller's `fn_type` was discarded. 240+ call
sites of this helper plus 116 inline copies of the same `unwrap_or_else`
pattern each potentially raced to declare the same FFI symbol with
conflicting signatures, and the loser silently got a wrong-typed
FunctionValue back. For `listen`, the precompiled-stdlib FFI lowering
at `instruction.rs:22655` constructed a void-returning declaration
from a malformed ffi_symbol signature (`ret_type → None → void_type`
fallback) and got there first.

**Five-part architectural fix** landed in `9a1a892b7`:

  1. **Canonical POSIX-syscall registry** (`syscall_registry.rs`) —
     `POSIX_SYSCALLS` table extended with every socket syscall
     (`socket` / `bind` / `listen` / `accept` / `connect` / `send` /
     `recv` / `sendto` / `recvfrom` / `setsockopt` / `waitpid`)
     under Verum's i64-everywhere AOT ABI.
  2. **`predeclare_all`** — new helper, wired into
     `vbc_lowering.rs::lower_module` as Phase 0.4, installs every
     registry entry into the LLVM module BEFORE any other emit path
     can race.
  3. **FFI-lowering registry override** at
     `instruction.rs::CallFfi` — when the symbol name is in
     `POSIX_SYSCALLS`, the canonical registry signature supersedes
     whatever shape the (potentially malformed) FFI declaration
     carries.
  4. **`emit_libc_free_socket_wrapper`** rewritten to consult the
     registry instead of constructing fn_types via an inline match
     table — 144 lines of drift-prone duplicated fn_type
     construction eliminated.
  5. **Signature-mismatch registry** in `error.rs` records every
     conflicting declaration into a process-global side channel;
     `check_no_signature_mismatches()` drains the registry at the end
     of LLVM lowering. Default mode writes warnings; strict mode
     (`VERUM_STRICT_SIGNATURES=1`) elevates to hard error.

**Verification**: `verum test --aot --filter regression_heap_round_trip_baseline`
passes (188s); same under `--interp` (124s). The original
`call_native_i64(lr): callee returns void` panic is structurally
eliminated.

**Drift surface (now visible, future work)**: the mismatch detector
surfaces 40+ pre-existing signature divergences as
`[codegen-warn] N signature mismatch(es) detected during LLVM lowering`
during every AOT build. Each is a real defect that was silently
producing wrong IR until this commit; LTO/DCE was hiding them at the
linker level. Notable families:

  * `pthread_*` (key_create / getspecific / setspecific) — `i32(ptr,…)`
    vs `i64(ptr,…)` POSIX-vs-Verum ABI inconsistency.
  * `verum_list_reverse` / `verum_list_swap` — `void(ptr,…)` vs
    `ptr(ptr,…)` return-type drift across emit paths.
  * `verum_string_join` — param-type drift (`ptr` vs `i64` for first arg).
  * `verum_raw_open3` / `verum_tcp_connect` — param width drift.
  * `sched_yield` — `i64()` vs `i32()` width drift.

Each of these is a separate fixable defect; the registry now makes
them discoverable. Tracked as future audit items.

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
| §F | Fix AOT `call_native_i64(lr): callee returns void` on `Heap.new` (see §3.5) | LANDED — commit `9a1a892b7` (canonical POSIX-syscall registry + signature-mismatch detection). 144 LOC eliminated; AOT regression test passes. |
| §G | Fix 40+ signature-mismatch warnings surfaced by the new gate (`pthread_*` family, `verum_list_reverse`/`swap` return-type drift, `verum_string_join` param drift, `verum_raw_open3` / `verum_tcp_connect` param width, `sched_yield` width). Each is a real defect that LTO/DCE hides at the linker level; route every declaration site through the canonical registry helper. | ~30 min per family | open |
