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
| 3 | property_test.vr — OBJ_* / FILE_* pairwise distinctness + OR-composition idempotence | Deferred until the broader stdlib base/protocols/Eq property suite ships a generic property runner. |
