# `core.sys.windows` — implementation audit (umbrella module)

## Status: **complete** (under `--interp`; re-export surface)

* Acts as the entry-point for the eight submodules: `ntstatus`,
  `ntdll`, `kernel32`, `tls`, `io`, `thread`, `time`, `winsock2`.
* Re-exports every public name through the umbrella so callers can
  `mount core.sys.windows.{Handle, NtStatus, INFINITE, ...}` without
  knowing about the submodule structure.

## 1. Cross-stdlib usage

The umbrella module is the documented entry-point for every Windows
caller inside / outside stdlib.  Direct imports against
`core.sys.windows.kernel32` / `core.sys.windows.ntdll` / etc. are
supported as escape hatches but not the canonical surface.

## 2. Action items landed in this branch

1. `unit_test.vr` — 21 `@test`s pinning that every documented
   re-export path resolves at compile time and produces the expected
   value at runtime via the umbrella name.  A regression that drops
   any re-export from `core/sys/windows/mod.vr` would cause the
   corresponding `@test` to fail at compile time. (Fixed the over-wide
   `TCB_MAGIC` literal typo at line 130 — see tls/audit.md / INTLIT-OVERFLOW-1.)
2. **`property_test.vr` (NEW, 2026-05-29)** — algebraic laws over the
   umbrella surface (kernel32 sentinels, TLS power-of-two constants,
   time scaling, winsock partitions, INVALID_HANDLE_VALUE all-ones) —
   doubles as a re-export-integrity guard.
3. **`integration_test.vr` (NEW)** — host-safe cross-submodule scenarios:
   NtStatus classification through `List`, access-mask union, AF routed
   through a `Map`. (The kernel32+ntdll+winsock2 *FFI* composition still
   needs a Windows runner.)
4. **`regression_test.vr` (NEW)** — umbrella LOCK-IN pins: canonical
   `TCB_MAGIC` via umbrella, all-ones sentinels, Windows divergences,
   STD_INPUT_HANDLE.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Cross-submodule *FFI* integration scenarios | Composing kernel32 + ntdll + winsock2 live calls needs a Windows test runner. |
| 2 | Function re-exports (`CreateFileW`, `ReadFile`, `socket`, …) | Cannot exercise on a non-Windows host; deferred to integration suite. |
