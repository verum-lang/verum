# `core.sys.windows.thread` — implementation audit

## Status: **partial** (under `--interp`; ADT + constant surface, runtime ops deferred)

* Provides Windows-side thread + synchronization primitives — thread
  creation/join, WindowsMutex (via Mutant), WindowsCondvar (via Event),
  WindowsSpinLock (atomic-based), WindowsOnce one-time initialiser,
  Win32Semaphore, and a futex_wait/wake helper layered on WaitOnAddress.
* The kernel-facing operations (`spawn`, `join`, `WindowsMutex.lock`,
  `WindowsCondvar.wait`, `futex_wait`, …) require kernel32.dll +
  WaitOnAddress / SetEvent / WaitForSingleObject and cannot run on a
  non-Windows host.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.async` | Task spawn / join routes via `spawn` + `WindowsThread.join`. |
| `core.sync.mutex` | The shared `Mutex<T>` impl delegates to `WindowsMutex` on Windows. |
| `core.sync.condvar` | `Condvar` delegates to `WindowsCondvar`. |
| `core.sync.once` | `Once` delegates to `WindowsOnce` (3-state atomic CAS). |

## 2. Pinned invariants

| Constant | Value | Why pinned |
|---|---|---|
| `DEFAULT_THREAD_STACK_SIZE` | 1 MiB | Matches Windows main-thread stack reservation default; drift would cause stack-allocated buffers to silently overflow. |
| `ONCE_INIT` | 0 | Initial state of a WindowsOnce — the "never been called" sentinel. |
| `ONCE_RUNNING` | 1 | Intermediate state — exactly one caller is inside the initialiser. |
| `ONCE_COMPLETE` | 2 | Terminal state — initialiser ran successfully. |

The triple `(ONCE_INIT, ONCE_RUNNING, ONCE_COMPLETE)` MUST be pairwise
distinct values for the CAS-based one-time initialisation pattern to
work — any drift would corrupt the WindowsOnce state machine.

## 3. Action items landed in this branch

1. `unit_test.vr` — 19 `@test`s pinning:
   * `DEFAULT_THREAD_STACK_SIZE` == 1 MiB;
   * Once state constants + pairwise distinctness;
   * All 11 `WindowsThreadError` variants — payload round-trip via
     match destructure for the 8 code-bearing variants, plus
     unit-payload variants (`TlsInitFailed`, `AlreadyJoined`,
     `AlreadyDetached`);
   * `message()` textual stability for the unit-payload variants.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `spawn` / `join` / `detach` round-trip | Requires kernel32 CreateThread / WaitForSingleObject. |
| 2 | `WindowsMutex` / `WindowsCondvar` / `WindowsSpinLock` / `WindowsOnce` runtime semantics | Requires Windows host. |
| 3 | `futex_wait` / `futex_wake` (WaitOnAddress / WakeByAddressSingle) | Requires Windows ≥ 8 host. |
| 4 | property_test.vr — Once state machine: INIT → RUNNING → COMPLETE monotone | Gated on §2. |
