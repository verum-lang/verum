# `intrinsics/memory` audit

Module: `core/intrinsics/memory.vr` (~357 LOC) — the unsafe raw-memory
primitive layer: in-place swap/replace, bit reinterpretation, raw pointer
read/write/offset, bulk memcpy/memset/memcmp, raw slices, uninit/zeroed, and
reference conversion.  ~45 public functions.

Tests: `unit_test.vr` + `regression_test.vr` cover the **value-level** subset
(no fabricated allocation).  The raw-pointer surface is deferred to a dedicated
allocation-based harness — see §3.

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

## 3. Defects OPEN / deferred

### MEM-RAWPTR-HARNESS-1 — raw-pointer surface needs an allocation harness  (task #23)

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
