# `core.sys.windows.tls` — implementation audit

## Status: **partial** (under `--interp`; constant + ADT surface, TCB / TLS-slot path deferred)

* Provides the Windows-side thread-local-storage bootstrap consumed by
  the V-LLSI runtime kernel: the `WindowsTlsError` ADT, the
  `ThreadControlBlock` / `ContextEntry` / `ContextSlots` types, the
  initialization sequence (`init_tls_index`, `init_main_thread_tls`,
  `create_thread_tls`, `cleanup_thread_tls`), the TCB accessors
  (`get_current_tcb`, `get_errno`, `set_errno`, `get_tid_fast`, …),
  and the context-slot push/pop/get/set primitives that the
  context system (`using [...]`) compiles to.
* `TlsAlloc` / `TlsGetValue` / `TlsSetValue` route via
  `core.sys.windows.kernel32` and require the kernel32.dll FFI;
  cannot be exercised on a non-Windows host.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.context` | The `using [Logger, Database]` runtime context machinery compiles to `ctx_push` / `ctx_pop` / `ctx_get` against the slot array. |
| `core.async` | Task spawn / join restores TCB state across continuations using `set_user_data` / `get_user_data`. |
| `core.sys.windows.thread` | Thread creation calls `create_thread_tls` to initialise a new TCB on the child thread. |
| `core.sys.windows.io` | IOCP completion routines stamp errno via `set_errno`. |

## 2. Pinned invariants

| Constant | Value | Why pinned |
|---|---|---|
| `MAX_CONTEXT_SLOTS` | 256 | Cross-platform invariant matching Linux + Darwin.  The context-system codegen statically reserves slot indices in [0, 256); any drift would silently corrupt context lookups. |
| `CONTEXT_STACK_DEPTH` | 8 | Maximum push-depth per context slot.  Drift would corrupt the `using` block exit unwinding. |
| `TCB_MAGIC` | 0x5645525545_4D5F_5443 ("VERUM_TC") | Marker the bootstrap kernel reads via `check_stack_guard` to validate that a `&TCB` actually points at a TCB.  Drift would cause the kernel to silently treat arbitrary memory as a valid TCB. |
| `DEFAULT_STACK_SIZE` | 1 MiB | Matches the default Windows main-thread stack reservation. |
| `GUARD_PAGE_SIZE` | 4 KiB | Single x86_64 page; the guard page traps stack overflow. |

## 3. Action items landed in this branch

1. `unit_test.vr` — 15 `@test`s pinning the five constants, the 8
   variants of `WindowsTlsError` (including payload-bearing variants
   `AllocationFailed { code, size }`, `InvalidSlot { slot }`,
   `StackOverflow / StackUnderflow { slot }`), and the Display
   textual output for the surface-facing variants.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | TCB layout + magic round-trip via FFI | Requires Windows host with kernel32.dll. |
| 2 | TLS-index lifecycle (init_tls_index / cleanup_thread_tls / TlsAlloc / TlsFree) | Same gating. |
| 3 | Context-slot push/pop/get/set semantics | Pinned at the source level via `core.context` tests; the Windows-specific slot-array storage path is gated on §1. |
| 4 | property_test.vr — `ctx_push` / `ctx_pop` LIFO invariant | Gated on §3. |
