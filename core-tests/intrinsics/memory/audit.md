# `intrinsics/memory` audit

Module: `core/intrinsics/memory.vr` (~357 LOC) — the unsafe raw-memory
primitive layer: in-place swap/replace, bit reinterpretation, raw pointer
read/write/offset, bulk memcpy/memset/memcmp, raw slices, uninit/zeroed, and
reference conversion.  ~45 public functions.

Tests: `unit_test.vr` + `regression_test.vr` (value-level subset) +
`property_test.vr` + `integration_test.vr` (2026-07-15: the raw-pointer
surface over the LIVE `cbgr_allocate` bridge — MEM-RAWPTR-HARNESS-1 (#23)
CLOSED by the same harness the mem_raw suite proved).  The 2026-07-15
campaign probed the full surface and split it into: WORKING (pointer
algebra, bulk ops after MEM-BULK-ADDR-DUAL-1), and three OPEN Tier-0
defect classes pinned as @ignore'd acceptance tests — see §2b/§3.

## 0. Architectural model (load-bearing)

This is the layer the CBGR safety system (`mem/*`) and `base/memory` are built
on.  Most operations take/return raw `*const T` / `*mut T` and require a **live
allocation** to exercise; they cannot be driven from pure values.  The
value-level subset — `swap`/`replace` (on `&mut` locals), `transmute` (bit
reinterpret), the `null_ptr` sentinel, and `zeroed` — IS exercisable in
isolation and is what this suite pins.

## 1. What is verified GREEN (interp)

* **swap** — exchanges two `&mut` locals; involution.
* **replace** — returns old, stores new.
* **transmute** — bit reinterpret (`UInt64`↔`Int` round-trip).
* **null_ptr / null_ptr_mut / ptr_is_null** — null sentinel + null check
  (after the §2 fix).
* **zeroed** — zero-initialisation (`zeroed::<Int>() == 0`).

## 2. Defects FIXED on this branch

### MEM-NULLPTR-1 — `null_ptr` / `ptr_is_null` returned `nil`

`null_ptr`'s registry strategy was `DirectOpcode(Opcode::LoadI)` — but `LoadI`
needs an immediate operand the DirectOpcode path never supplies, so it fell
through to `LoadNil`.  `ptr_is_null`'s strategy was `DirectOpcode(Opcode::EqI)`
— a *binary* compare invoked with a single operand (the pointer), which also
fell through to `LoadNil`.

**Fix**: dedicated inline sequences (`InlineSequenceId::NullPtr` →
`LoadI dst, 0`; `InlineSequenceId::PtrIsNull` → `LoadI tmp, 0` + `CmpI Eq dst,
ptr, tmp`).  Re-pointed `null_ptr`/`null_ptr_mut`/`ptr_is_null` (+ their
`intrinsic_*` aliases) to them.

## 2b. Defects FIXED 2026-07-15 (session campaign)

### MEM-BULK-ADDR-DUAL-1 — Tier-0 bulk ops silently no-op'd int-tagged buffers

The interpreter's CMemcpy/CMemset/CSecureZero/CMemmove/CMemcmp handlers
(`handlers/ffi_extended.rs`) extracted buffer addresses with bare
`Value::as_ptr()`.  Raw addresses legitimately arrive EITHER pointer-tagged
OR int-tagged (`cbgr_allocate` bridge, `as *mut T` casts — the
RAWPTR-DROPREF-1 int-tagging); an int-tagged address decoded as NULL and
the null-guard silently skipped the operation — every
`core.intrinsics.memory` bulk op over a bridge buffer was a Tier-0 no-op
while AOT executed it (cross-tier divergence), and memcmp ranked garbage
(two equal buffers compared -1).  FutexWait/FutexWake/SpinlockLock had the
mirror int-only extraction.  **Fix**: the canonical `value_as_addr` dual
extraction (the helper the PtrAdd family and atomic handlers already used)
across all eight handlers.  Pass-guards in regression_test.vr; laws in
property_test.vr §6; two-module agreement in integration_test.vr.

## 3. Defects OPEN / deferred

### MEM-PTR-DEREF-TIER0-1 — ptr_read/ptr_write width channel  (task #16, 2026-07-15)

Registry binds ptr_read → `DirectOpcode(Deref 0x72)` / ptr_write →
`DerefMut (0x73)`; the CBGR handlers treat a raw int-tagged address as the
un-dereferenceable FOURTH shape (identity / silent no-op) while AOT loads
and stores correctly — a cross-tier kernel incident.  Pointer ARITHMETIC is
Value-slot-strided (×8) for every pointee (`ptr_add(p: *const Byte, 1)`
advances 8; the contract is `count × T.size`).  Fix design (task #16):
generalize the EXISTING `ptr_elem_stride` channel
(`emit_intrinsic_instructions` → `emit_intrinsic_inline_sequence`) to full
pointee width {1,2,4,8} + signedness derived at the call site, route
PtrRead/PtrWrite to `DerefRaw`/`DerefRawSigned`/`DerefMutRaw {size}` (the
handlers already do dual extraction) and scale the arith arms.  @ignore'd
acceptance pins in property/regression.

### MEM-SLICE-INTRINSIC-FATREF-1 — slice family SIGSEGV / ghost symbols  (task #17, 2026-07-15)

`slice_len`/`slice_as_ptr` emit `Unpack {count: 2}` treating a slice as a
2-tuple heap object — a slice value is ONE FatRef; Unpack misreads it and
SIGSEGVs the interpreter.  `slice_get[_unchecked]`/`slice_subslice`/
`slice_split_at` route to `verum_slice_*` library symbols defined nowhere
(the ghost-symbol drift class named in the adjacent TextParse comment).
Fix: route through the canonical FatRef slice-cell view (the #48-campaign
authority).  @ignore'd pins — DO NOT un-ignore before the fix (SIGSEGV).

### MEM-PTR-ALIGN-NIL-1 — ptr_is_aligned/_to return nil  (task #18, 2026-07-15)

Same LoadNil registry class as MEM-NULLPTR-1.  `ptr_is_aligned_to` is
width-free (`(ptr & (align-1)) == 0` sequence); `ptr_is_aligned<T>` needs
the #16 width channel for `T.alignment`.

### MEM-RAWPTR-HARNESS-1 — raw-pointer surface needs an allocation harness  (task #23 — CLOSED 2026-07-15)

CLOSED by property/integration over the `cbgr_allocate` bridge (the exact
harness mem_raw proved).  Kept for history; the remaining gaps above are
op-level defects, not harness gaps.

The bulk of the module operates on raw pointers and cannot be tested without a
live allocation:

* `ptr_read` / `ptr_read_unaligned` / `ptr_read_volatile`,
  `ptr_write` / `_unaligned` / `_volatile` / `ptr_write_bytes`
* `ptr_offset` / `ptr_offset_mut` / `ptr_add` / `ptr_sub`,
  `ptr_is_aligned` / `ptr_is_aligned_to`, `ptr_to_ref` / `ptr_to_mut_ref`
* `memmove` / `memcpy` / `memset` / `memcmp`, `copy` / `copy_nonoverlapping`,
  `volatile_copy` / `volatile_set`
* `slice_from_raw_parts[_mut]` / `slice_len` / `slice_as_ptr` /
  `slice_as_mut_ptr` / `slice_get_unchecked[_mut]` / `slice_subslice[_mut]` /
  `slice_split_at[_mut]`
* `uninit` / `maybe_uninit_is_init`, `forget`, `drop_in_place`

These need a harness that obtains a real `*const T`/`*mut T` (from a `List`/slice
backing or the `mem` allocator) and round-trips through each op, on both tiers.
Partly covered already by `core-tests/base/memory` + `core-tests/mem/*` (the
CBGR safety layer).  Deep / delicate (unsafe); a focused follow-up.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.*` (CBGR allocator, arenas, refs) | the entire raw read/write/offset + memcpy surface. |
| `core.collections.*` | raw slice ops + `copy_nonoverlapping` for growable buffers. |
| `core.text` | `memcpy`/`memcmp` for byte-buffer manipulation. |
| `base/memory` | `swap`/`replace`/`transmute` for value movement. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the memory intrinsic entries;
  the `NullPtr`/`PtrIsNull` inline-sequence strategies.
* `crates/verum_vbc/src/codegen/expressions.rs::emit_intrinsic_inline_sequence`
  — `NullPtr` (`LoadI 0`) / `PtrIsNull` (`LoadI 0` + `CmpI Eq`) emission.
* `crates/verum_vbc/src/interpreter/` + `crates/verum_codegen/src/llvm/` — the
  raw `ptr_*` / `mem*` / `slice_*` opcode semantics (the §3 surface).

## 6. Action items

**Landed this branch**
* MEM-NULLPTR-1 — `null_ptr`/`ptr_is_null` via dedicated inline sequences.
* Value-level memory test suite (unit + regression).

**Deferred (tracked)**
* MEM-RAWPTR-HARNESS-1 (#23) — allocation-based harness for the raw-pointer /
  slice / memcpy surface, both tiers.
