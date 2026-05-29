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

2. `property_test.vr` — 14 `@test`s (`wx_thread_*`) over the pure surface:
   * Once states consecutive 0,1,2 + pairwise-distinct (list sweep) +
     strictly increasing (CAS-monotone invariant);
   * `DEFAULT_THREAD_STACK_SIZE` == 1 MiB, power-of-two, == 2^20;
   * `WindowsThreadError` Eq reflexivity (all 8 code + 3 unit variants);
   * Eq payload-sensitivity (different code ⇒ ≠);
   * Eq variant-discrimination (same code, different tag ⇒ ≠; unit vs
     code variant ⇒ ≠);
   * `message()` totality (non-empty for every variant) + injectivity
     across the three unit variants;
   * `is`-operator tag disjointness.

3. `integration_test.vr` — 10 `@test`s (`wx_thread_*`):
   * `WindowsThreadError` flowing through `List` with a `match`-based
     recoverable/non-recoverable classifier;
   * code extraction into `Maybe<UInt32>` (Some for code variants, None
     for unit variants);
   * Once-state set dedup → exactly 3, min/max span = (INIT, COMPLETE);
   * `Map<UInt32, Text>` keyed by Once state → label;
   * stack-size scaling stays power-of-two; `Maybe<USize>` pipeline.

4. `regression_test.vr` — 7 `@test`s (`wx_thread_lockin_*`) pinning
   fragile pure invariants:
   * Once sentinel triple exact literals (0,1,2) + distinctness;
   * `DEFAULT_THREAD_STACK_SIZE` exact byte count;
   * `code` payload round-trip at `0xFFFFFFFF` boundary (no narrowing)
     + `ERROR_TIMEOUT` (1460) survives;
   * Eq does not collapse distinct variants sharing a payload;
   * `message()` byte-stability for unit variants.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `spawn` / `spawn_suspended` / `join` / `join_timeout` / `detach` / `suspend` / `resume` round-trip | Requires kernel32 CreateThread / WaitForSingleObject / SuspendThread / ResumeThread. |
| 2 | `WindowsMutex` / `WindowsCondvar` / `Win32Semaphore` runtime semantics | Requires Windows host (CreateMutexW / CreateEventW / CreateSemaphoreW + WaitForSingleObject). |
| 3 | `WindowsSpinLock` lock/try_lock/unlock/is_locked | Pure constructor `new()` returns `{ locked: 0 }` but the lock ops drive `atomic_cas_u32` / `atomic_store_u32` / `atomic_load_u32` intrinsics on shared mutable state — not FFI but not a static-value pin; deferred to a concurrency-capable harness. |
| 4 | `WindowsOnce.call_once` / `is_complete` runtime semantics | Requires atomic CAS + Event FFI; INIT→RUNNING→COMPLETE monotone pinned statically in property_test instead. |
| 5 | `current_tid` / `current_thread_id` / `yield_now` | Wrap GetCurrentThreadId / Sleep — FFI. |
| 6 | `futex_wait` / `futex_wake` (WaitOnAddress / WakeByAddressSingle/All) | Requires Windows ≥ 8 host. Note: the ns→ms ceiling-division arithmetic inside `futex_wait` is pure but unreachable without the WaitOnAddress call surrounding it, so it cannot be unit-pinned in isolation. |
| 7 | `WindowsThread` / `WindowsMutex` / `WindowsCondvar` / `Win32Semaphore` struct field-layout pins | Constructible only via FFI-returning factories; no pure ctor to pin field defaults. |

## 5. Suspected defects

None observed in the pure surface. All ADT / constant / Eq / message()
invariants behaved as declared during authoring (not executed here per
instructions — DO NOT run build/test was honored).
