# `core.sys.windows.kernel32` — implementation audit

## Status: **partial** (under `--interp`; constant + struct surface, FFI deferred)

* Provides ~150 Win32 API constants (memory protection, file
  attributes/flags, creation disposition, error codes, wait codes,
  notify-change codes, file actions, …) plus IAT-imported function
  signatures for the entire kernel32.dll surface used by the runtime
  (file I/O, threads, synchronization, IOCP, TLS, time, console).
* The FFI bindings (`CreateFileW`, `ReadFile`, `WaitForSingleObject`, …)
  cannot be exercised on a non-Windows host — the kernel32.dll DLL does
  not exist and the loader rejects the IAT entry.  The constant + struct
  surface IS portable and is what we pin here.
* Reference: Microsoft Win32 API headers (winnt.h, winbase.h, fileapi.h,
  errhandlingapi.h, synchapi.h).

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.sys.windows.io` | `CreateIoCompletionPort` / `GetQueuedCompletionStatusEx` / `PostQueuedCompletionStatus` are the IOCP backbone. |
| `core.sys.windows.thread` | `CreateThread` / `WaitForSingleObject` / `SRWLock` / `ConditionVariable` / `WaitOnAddress` are the synchronization primitives. |
| `core.sys.windows.tls` | `TlsAlloc` / `TlsGetValue` / `TlsSetValue` are the slot operations the bootstrap TCB consumes. |
| `core.sys.windows.time` | `QueryPerformanceCounter` / `GetTickCount64` / `GetSystemTimeAsFileTime` drive monotonic + wall clocks. |
| `core.io.fs` | `CreateFileW` / `ReadFile` / `WriteFile` / `CloseHandle` / `LockFileEx` are the file-system backend. |
| `core.sys.process_native` | `CreateProcessA` / `CreatePipe` / `WaitForSingleObject` are the process backbone. |

## 2. Action items landed in this branch

1. `unit_test.vr` — ~75 `@test`s pinning the canonical bit-level value
   of every documented constant: MEM_*, PAGE_*, STD_*_HANDLE, FILE_ATTRIBUTE_*,
   FILE_FLAG_*, CREATE_*/OPEN_*/TRUNCATE_*, ERROR_*, WAIT_*, INFINITE,
   FILE_NOTIFY_CHANGE_*, FILE_ACTION_*, FILE_BEGIN/CURRENT/END,
   LOCKFILE_*, STARTF_USESTDHANDLES, CREATE_NO_WINDOW, HANDLE_FLAG_INHERIT,
   STILL_ACTIVE, CREATE_SUSPENDED, TLS_OUT_OF_INDEXES.
2. `property_test.vr` — 17 algebraic laws covering:
   * MEM_* / PAGE_* / FILE_FLAG_* / FILE_NOTIFY_CHANGE_* are pairwise
     distinct (callers can safely OR-combine them);
   * MEM_* / PAGE_* / FILE_FLAG_* / FILE_NOTIFY_CHANGE_* are powers of
     two (each occupies a single bit position);
   * Standard handles are exactly the three documented two's-complement
     negatives (-10/-11/-12 as UInt32);
   * Win32 error codes are pairwise distinct;
   * Creation disposition codes are consecutive 1..=5 (enum-safe);
   * FILE_ACTION_* are consecutive 1..=5 (enum-safe);
   * WAIT_FAILED ≡ INFINITE (both 0xFFFFFFFF; disambiguation by context).
3. `regression_test.vr` — 3 `@test`s pinning defect classes (see §3).
4. `integration_test.vr` — 8 cross-stdlib scenarios — file-flag OR
   composition, page-protection escalation table, wait-code Result funnel,
   error-class dispatch table, notify-change subscription mask, and
   creation-disposition decision tree.

## 3. Defects

### §A — UInt32 const initialiser comparison drift **[CLOSED]**

**Symptom.** Constants like `FILE_FLAG_WRITE_THROUGH: UInt32 = 0x80000000`
silently mis-compared with the literal `0x80000000_u32` at the call site,
because the const's stored value was sign-extended through the 48-bit Int
payload at codegen time.

**Closed by.** The same VBC codegen sign-extension fix that closes
`core.sys.windows.ntstatus` §A — see
`core-tests/sys/windows/ntstatus/audit.md §A` for the full diagnostic
chain.  UInt32 constants now round-trip through the runtime intact.

### §B — Bitmask-safety contract pinned

**Pinned by.** `prop_*_are_powers_of_two` / `prop_*_pairwise_distinct`.

**What it pins.** Every constant group designed for OR-combination
(MEM_*, PAGE_*, FILE_FLAG_*, FILE_NOTIFY_CHANGE_*) MUST be a power of
two AND pairwise distinct.  An accidental rename of a constant to a
sibling's value would silently mask bits at every caller — these
properties make that regression visible immediately.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | FFI bindings (`CreateFileW` / `ReadFile` / `WaitForSingleObject` / …) | Cannot exercise on non-Windows host.  AOT cross-compile to Windows + run under Wine / a Windows VM is the only path; deferred. |
| 2 | `Overlapped` struct round-trip via FFI | Pinned shape (`new()`, `with_event()`, `set_offset()`, `get_offset()`) at the Verum level; the FFI marshalling layer is gated on §1. |
| 3 | `SystemInfo` / `WindowsSystemTime` / `StartupInfoA` / `ProcessInformation` field layouts | Pinned at the struct level for Verum codegen; the C-ABI layout match is gated on §1. |
