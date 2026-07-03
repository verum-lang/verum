# `intrinsics/runtime/cbgr` audit

Module: `core/intrinsics/runtime/cbgr.vr` (~180 LOC) — CBGR memory-safety
intrinsics: reference validation, epoch/generation management, and the
allocation/deallocation bridge (#67 Phase 3 Tier A).

Tests: unit (14) + property (3) + integration (4) + regression (2) over the
value-level surface: `cbgr_allocate` / `cbgr_deallocate` / `cbgr_realloc`,
`cbgr_validate`, `cbgr_get_epoch` / `cbgr_current_epoch`.

## 0. Architectural model (load-bearing)

The module carries TWO API layers that were never reconciled until this
audit:

* **Allocator-internal layer** (`cbgr_alloc(size, align)` → `Result<(ptr,
  gen, epoch)>`, 4-arg `cbgr_realloc`, 3-arg `cbgr_dealloc`) — declared in
  `core/mem/allocator.vr`, consumed by `base/memory`, `Heap`, `Shared`,
  collections.  Wired via `InlineSequence` → `FfiExtended 0xA0-0xA2`.
* **Public user-pointer bridge** (`cbgr_allocate(size, align) -> Int`,
  `cbgr_deallocate(ptr)`, `cbgr_realloc(ptr, new_size) -> Int`) — declared
  HERE.  Until 2026-07-03 this layer was **link-surface only**: no registry
  entries, no interpreter handlers, no AOT lowering arms.

## 1. Defects FIXED on this branch (2026-07-03)

### CBGR-REALLOC-PTRADD-1 — realloc inline sequence emitted sub-op 0x63 = PtrAdd

`InlineSequenceId::CbgrRealloc` emitted `FfiExtended 0x63` — which decodes
as `SystemSubOpcode::PtrAdd`.  Every inline-sequence reallocation either
SIGSEGV'd (interpreter: 4-operand handler over a 2-operand encoding) or
silently computed `ptr + old_size` (AOT).  The 0x60-0x62 legacy-stub
collision was fixed for alloc/dealloc previously; realloc was missed.
**Fix**: dedicated `SystemSubOpcode::CbgrRealloc = 0xA4` + interpreter
handler (fresh alloc, `min(old, new)` copy, `Ok((ptr, gen, epoch))`
packaging) + AOT arm calling `verum_cbgr_realloc`.

### CBGR-BRIDGE-HOLLOW-1 — the public bridge resolved to nothing

`cbgr_allocate` / `cbgr_deallocate` had NO registry entries (calls compiled
to nil-producing fallbacks; tests passed vacuously via nil arithmetic), and
the bridge's `cbgr_realloc` key COLLIDED with the internal 4-arg entry.
**Fix**: first-class wiring — registry entries + `FfiExtended 0xA5-0xA7` +
interpreter handlers implementing the AOT runtime's **32-byte
AllocationHeader model** (size@0, align@4, generation@8, epoch@12,
flags@20; reserved@24 stores `{base_offset, total}` making each block
self-describing) + AOT arms calling `verum_cbgr_allocate` /
`verum_cbgr_deallocate` / `verum_cbgr_realloc`.  Bridge realloc key renamed
to `cbgr_realloc_user` (the .vr fn name stays `cbgr_realloc`).

### CBGR-VALIDATE-SHAPE-1 — `cbgr_validate` returned an unset register

Strategy was `DirectOpcode(ChkRef)`; `ChkRef` validates-or-PANICS and
writes no result, so `let ok = cbgr_validate(&x)` read an unset register —
false for every live reference.  Same `DirectOpcode`-shape-misuse family as
MEM-NULLPTR-1.  **Fix**: `FfiExtended 0xA8 CbgrValidateBool` — the same
validation logic, non-trapping, writing the Bool verdict
(`handlers/cbgr.rs::validate_ref_bool`).  AOT arm: null-check (Tier-2
references are compiler-checked; the residual dynamic question is
null-ness).

## 2. Defects OPEN / limitations (documented contract)

* **Alignment cap**: the AOT runtime `verum_cbgr_allocate(size)` does not
  receive the align argument; alignment beyond the header-natural 32 is not
  guaranteed.  Property sweep pins 1..32.  Follow-up: thread `align`
  through the runtime signature.
* **Use-after-free detection window**: `cbgr_deallocate` really frees (both
  tiers), so a stale user pointer is not detectable afterwards — matching
  AOT.  The FREED flag is set before release for the tracked window only.
* **Global-state mutators untested by design**: `cbgr_advance_epoch`,
  `cbgr_epoch_begin`, `cbgr_new_generation`, `cbgr_invalidate`,
  `cbgr_revoke`, `cbgr_register_root` mutate process-global CBGR state; an
  in-process test runner would leak their effects into sibling tests.
  Needs a subprocess-isolated harness (deferred).
* **Packed-word validators untested**: `cbgr_validate_ref` / `cbgr_check` /
  `cbgr_check_fat` / `cbgr_check_write` take runtime-internal packed
  layouts; exercised indirectly by the CBGR safety suites under
  `core-tests/mem/`.

## 3. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.allocator` | the internal 0xA0-0xA2 layer (this module's sibling API). |
| `base/memory`, `Heap`, `Shared` | allocation via `cbgr_alloc`; validation at deref. |
| `core-tests/intrinsics/runtime/mem_raw` | the bridge is the allocation harness for the raw byte/word suite. |

## 4. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/instruction.rs` — `SystemSubOpcode` 0xA0-0xA8 block.
* `crates/verum_vbc/src/intrinsics/registry.rs` — the two API layers' entries;
  the `cbgr_realloc` vs `cbgr_realloc_user` key split.
* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/ffi_extended.rs`
  — `cbgr_user_allocate/deallocate/realloc` (header model).
* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/cbgr.rs` —
  `validate_ref_bool` (shared with ChkRef semantics).
* `crates/verum_codegen/src/llvm/instruction.rs` — 0xA4-0xA8 lowering arms.
* `crates/verum_codegen/src/llvm/runtime.rs` — `verum_cbgr_*` runtime fns
  (the AOT header-model authority).
* `verum_common::layout::ALLOCATION_HEADER_*` — the single layout authority
  both tiers read.

## 5. Action items

**Landed this branch**
* CBGR-REALLOC-PTRADD-1, CBGR-BRIDGE-HOLLOW-1, CBGR-VALIDATE-SHAPE-1 (above).
* Full conformance suite for the bridge + validation + epoch reads.

**Deferred (tracked)**
* Runtime `verum_cbgr_allocate(size, align)` signature (alignment cap).
* Subprocess harness for global-state mutators.
* TYPEINFO-ID-CANON-1 interaction: `cbgr_get_generation` typed-pointer form.
