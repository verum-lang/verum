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
| `TCB_MAGIC` | 0x5645_5255_4D5F_5443 ("VERUM_TC", 64-bit; see §5) | Marker the bootstrap kernel reads via `check_stack_guard` to validate that a `&TCB` actually points at a TCB.  Drift would cause the kernel to silently treat arbitrary memory as a valid TCB. |
| `DEFAULT_STACK_SIZE` | 1 MiB | Matches the default Windows main-thread stack reservation. |
| `GUARD_PAGE_SIZE` | 4 KiB | Single x86_64 page; the guard page traps stack overflow. |

## 3. Action items landed in this branch

1. `unit_test.vr` — 15 `@test`s pinning the five constants, the 8
   variants of `WindowsTlsError` (including payload-bearing variants
   `AllocationFailed { code, size }`, `InvalidSlot { slot }`,
   `StackOverflow / StackUnderflow { slot }`), and the Display
   textual output for the surface-facing variants.

2. `property_test.vr` — 23 `@test`s (`wx_tls_prop_*`): sentinel pins
   (`TLS_OUT_OF_INDEXES == 0xFFFFFFFF` / max-u32), `TCB_MAGIC`
   little-endian byte packing + non-zero invariant, power-of-two
   geometry (`GUARD_PAGE_SIZE`, `DEFAULT_STACK_SIZE`,
   `MAX_CONTEXT_SLOTS`, `CONTEXT_STACK_DEPTH`),
   `DEFAULT_STACK_SIZE > GUARD_PAGE_SIZE`, `WindowsTlsError` Eq
   reflexivity + pairwise-distinctness lattice + payload-sensitivity,
   and the `is_retryable() == false` uniform law over all 8 variants.

3. `integration_test.vr` — 14 `@test`s (`wx_tls_int_*`): `WindowsTlsError`
   in `List` / `Maybe` / `Result` / `Map`; pure host-side mirrors of the
   `ctx_get`/`ctx_set` slot-bounds predicate, the `ctx_push` overflow
   guard, and the `ctx_pop` underflow guard (truth tables, no FFI);
   8-way match-exhaustiveness tag-disjointness; `message()` non-empty law;
   constants as `Map` values.

4. `regression_test.vr` — 9 `@test`s (`wx_tls_reg_*`): LOCK-IN pins for
   `TCB_MAGIC` (exact 64-bit value + per-byte decomposition),
   `TLS_OUT_OF_INDEXES` all-ones, the cross-platform `MAX_CONTEXT_SLOTS` /
   `CONTEXT_STACK_DEPTH` ABI constants, page geometry, and
   `WindowsTlsError` discriminant disjointness for the same-shape
   `{ slot }` family + `AllocationFailed` two-field equality.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | TCB layout + magic round-trip via FFI | Requires Windows host with kernel32.dll. |
| 2 | TLS-index lifecycle (init_tls_index / cleanup_thread_tls / TlsAlloc / TlsFree) | Same gating — FFI into kernel32 `Tls*`. |
| 3 | Context-slot push/pop/get/set semantics | Live slot-array storage requires a TCB allocated via `VirtualAlloc` + `TlsSetValue`. Pure bounds/overflow/underflow *predicates* are now covered host-side in `integration_test.vr`; the FFI-backed storage path stays gated on §1/§2. |
| 4 | `ctx_push` / `ctx_pop` LIFO value-restore invariant | Needs a live TCB; only the depth-guard truth tables are host-testable. |
| 5 | TCB accessors (`get_current_tcb`, `get_errno`/`set_errno`, `get_tid_fast`, `check_stack_guard`, `get/set_user_data`, `is_tls_initialized`) | All route through `TlsGetValue` / `GetLastError` / `SetLastError` FFI; not host-safe. |
| 6 | `generate_stack_canary` | Calls `GetTickCount64` / `GetCurrentThreadId` / `GetCurrentProcessId` FFI; not host-safe. |

## 5. Discrepancy found — `TCB_MAGIC` literal in unit_test.vr / §2 table

`unit_test.vr:47` (and `core-tests/sys/windows/mod/unit_test.vr:130`) both
pinned `TCB_MAGIC` as `0x5645525545_4D5F_5443` (**18 hex digits = 72 bits**,
with a spurious extra `45` byte). That literal does not fit in a `UInt64`
and is **not** the value the source defines.

**RESOLVED 2026-05-29**: both test typos corrected to the source-correct
`0x5645_5255_4D5F_5443_u64`.

The authoritative source `core/sys/windows/tls.vr:49` declares:

```
public const TCB_MAGIC: UInt64 = 0x5645_5255_4D5F_5443;  // "VERUM_TC"
```

That is the correct 16-hex-digit (64-bit) little-endian packing of the
ASCII bytes `V E R U M _ T C` (`0x56 0x45 0x52 0x55 0x4D 0x5F 0x54 0x43`).

`regression_test.vr::wx_tls_reg_tcb_magic_exact_64bit` +
`wx_tls_reg_tcb_magic_byte_decomposition` pin the **source-correct** value.

**Language-level finding (INTLIT-OVERFLOW-1).** Confirmed by probe: the
72-bit literal is **silently wrapped mod 2⁶⁴** (the parser builds it as
`i128` then lowering narrows it) — it produces `4995148692846498883`
(its low 64 bits) with **no diagnostic**, rather than being rejected.
This is why the typo'd tests were silently *failing* (wrapped value ≠
real `TCB_MAGIC`) instead of erroring at compile time. Tracked as
**INTLIT-OVERFLOW-1** in
`internal/website/docs/stdlib/defect-class-catalogue.md §11`; fix is a
parse/type-check range guard on suffixed integer literals.
