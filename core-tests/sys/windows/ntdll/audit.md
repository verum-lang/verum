# `core.sys.windows.ntdll` — implementation audit

## Status: **partial** (under `--interp`; constant + Handle type surface, FFI deferred)

* Provides the NT Native API surface — the lowest-level userspace
  binding into the Windows kernel.  Exposes `Handle` (newtype Int with
  NULL / INVALID sentinels + is_valid + raw), `LargeInteger` /
  `ULargeInteger` newtype wrappers around 64-bit integers,
  `UnicodeString` / `ObjectAttributes` / `IoStatusBlock` ABI shapes,
  ~30 OBJ_* / GENERIC_* / FILE_* access-mask constants, and the
  `Nt*` IAT-imported functions (NtCreateFile, NtReadFile, NtClose,
  NtAllocateVirtualMemory, NtWaitForSingleObject, …).
* The Nt* FFI bindings cannot run on a non-Windows host.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.sys.windows.kernel32` | Re-exports `Handle` / `LargeInteger` / `IoStatusBlock` for higher-level wrappers. |
| `core.sys.windows.io` | IOCP handles use the `Handle` newtype. |
| `core.sys.windows.tls` | TLS allocation uses `NtAllocateVirtualMemory` (when bypassing kernel32). |
| `core.sys.windows.thread` | `NtCreateThreadEx` (when bypassing kernel32). |

## 2. Action items landed in this branch

1. `unit_test.vr` — 32 `@test`s pinning:
   * Handle: NULL.raw() == 0, INVALID.raw() == -1, is_valid for NULL /
     INVALID / arbitrary handle, raw round-trip;
   * OBJ_* object attribute flags (8 constants);
   * GENERIC_* / DELETE / READ_CONTROL / WRITE_DAC / WRITE_OWNER /
     SYNCHRONIZE (9 standard rights);
   * FILE_READ_DATA through FILE_WRITE_ATTRIBUTES (9 fine-grained
     file rights) + FILE_ALL_ACCESS / FILE_GENERIC_READ /
     FILE_GENERIC_WRITE / FILE_GENERIC_EXECUTE composites;
   * FILE_SUPERSEDE / FILE_OPEN / FILE_CREATE / FILE_OPEN_IF
     (4 NtCreateFile disposition codes).

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Nt* FFI round-trip (NtCreateFile / NtReadFile / NtAllocateVirtualMemory / …) | Requires Windows host with ntdll.dll. |
| 2 | UnicodeString / ObjectAttributes binary layout against the C ABI | Pinned at the Verum type-shape level; ABI match requires Windows host validation. |

## 2b. Landed this branch (2026-05-29)

* **`property_test.vr` (NEW)** — OBJ_* single-bit + pairwise-disjoint +
  `OBJ_VALID_ATTRIBUTES` == bitwise-union law; GENERIC_* / PAGE_* / MEM_* /
  fine-grained FILE_* single-bit + distinct; disposition codes consecutive
  0..=5; THREAD_CREATE_FLAGS_* single-bit + disjoint; `Handle.is_valid`
  ≡ `raw ∉ {0,-1}`; LargeInteger / ULargeInteger injective round-trip.
* **`integration_test.vr` (NEW)** — Handle filtering through `List`,
  Handle-in-`Maybe`, `IoStatusBlock.new()` zero-init contract, LargeInteger
  as `Map` value, GENERIC_READ|WRITE mask union.
* **`regression_test.vr` (NEW)** — LOCK-IN pins for OBJ_VALID_ATTRIBUTES
  0x7F2, GENERIC_* exact bits, FILE_GENERIC_* composite masks, disposition
  prefix, Handle NULL/INVALID sentinels.

## §B. Language defect — single-field-newtype method-return unboxing (NEWTYPE-UNBOX-1)

**Found 2026-05-29 while authoring `integration_test.vr`.**

`IoStatusBlock.status()` returns `NtStatus(self.status_or_pointer as Int32)`.
`NtStatus is (Int32)` is a single-field newtype. A `NtStatus` value *returned
from a method* unboxes to a bare `Int` at runtime, so chaining
`iosb.status().is_success()` panics with `NtStatus.is_success not found on
receiver of runtime kind Int`. Direct `NtStatus` values (the `STATUS_*`
consts) dispatch fine — only the method-returned newtype loses its type.

Same class as the core `Duration` single-field-record unboxing defect
(2026-05-27) and the `WindowsDuration.add().as_millis()` mis-read pinned in
`core-tests/sys/windows/time`. Catalogued as **NEWTYPE-UNBOX-1** in
`website:docs/stdlib/defect-class-catalogue.md §12`; deep fix is VBC
codegen newtype boxing/unboxing parity (tracked task). Working idiom:
`integration_test.vr` asserts `iosb.status_or_pointer == 0` and
`iosb.bytes_transferred() == 0` directly; `regression_test.vr` pins the broken
chain with an `@ignore`.
