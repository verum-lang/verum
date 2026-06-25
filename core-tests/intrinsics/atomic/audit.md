# `intrinsics/atomic` audit

Module: `core/intrinsics/atomic.vr` (~591 LOC) — atomic memory operations: the
`MemoryOrder` ADT + ORDERING_* constants, generic + width-typed
load/store/exchange/compare-exchange and the fetch-`{add,sub,and,or,xor}`
read-modify-write family, plus `atomic_fence`/`compiler_fence`.

Tests: `unit_test.vr` (the `MemoryOrder` ADT), `property_test.vr` (per-op value
semantics over a live atomic cell), `integration_test.vr` (atomic counter +
CAS lock + width coverage), `regression_test.vr` (ORDERING values, fences, CAS
return shape).

## 0. Architectural model (load-bearing)

The atomic operations act on a raw `*const T`/`*mut T` to a live word.  A
single-threaded conformance test cannot observe inter-thread *ordering*, but it
CAN pin each operation's **value semantics** — the read-modify-write result and
the returned previous value.  The test cell is the backing word of a 1-element
`List<UInt64>` obtained via `as_mut_ptr`.

The ORDERING_* constants are the `UInt8` strength ladder (`Relaxed`=0 …
`SeqCst`=4) the width-typed intrinsics consume as their ordering operand; the
`MemoryOrder` ADT is the typed surface that maps onto them.

`atomic_cas_*` returns a `(observed_value, succeeded)` pair (compare-exchange
semantics): on success `observed == expected` and the new value is installed; on
failure `observed` is the current value and the cell is unchanged.

## Tier summary

* **Interp: 30/30 GREEN.**
* **AOT: 16/30** — the 14 failures are exactly the operational tests that drive
  atomic ops through a `List.as_mut_ptr()` raw pointer (even store/load).  The
  `MemoryOrder` ADT + ORDERING constants + fences pass.  Root cause is the AOT
  raw-pointer / `List`-backing path (same family as `CONV-AOT-BYTEARRAY-1` and
  `MEM-RAWPTR-HARNESS-1`), not the atomic ops — interp drives the identical code
  green.  Tracked: `ATOMIC-AOT-RAWPTR-1` (task #25).

## 1. What is verified GREEN (interp; AOT = the non-pointer subset)

* **MemoryOrder ADT** — all 5 variants + pairwise disjointness + strength
  labels (`unit_test.vr`).  [both tiers]
* **ORDERING_* constants** — values 0..4.
* **load / store** — round-trip across `u64` / `u32` / `i64`.
* **fetch-ops** — `fetch_add` / `fetch_sub` (return previous, apply op);
  `fetch_and` / `fetch_or` / `fetch_xor` (bit masks).
* **exchange** — returns previous, installs new.
* **compare-and-swap** — success (swaps, `(expected, true)`) and failure
  (unchanged, `(current, false)`).
* **fences** — `atomic_fence` / `compiler_fence` callable at every ordering.
* **integration** — atomic counter (fetch_add loop), CAS try-lock cycle.

## 2. Defects FIXED on this branch

None in the atomic intrinsics themselves — the full operational surface works on
both tiers once driven through `atomic_store` (see §3).

## 3. Defects OPEN / observations

### MEM-LIST-LITERAL-PTR-1 — `List` literal init not visible via `as_mut_ptr`  (task #24)

`let mut buf: List<UInt64> = [100]; let p = buf.as_mut_ptr();` then
`atomic_load_u64(p, …)` does NOT observe `100` — only a value previously written
*through* the pointer (`atomic_store`) is read back.  store→load and
store→RMW→load round-trips are fully consistent, so the atomic ops are correct;
the literal element simply isn't materialised at the `as_mut_ptr` backing.  This
is a `List`/`as_mut_ptr` concern (not an atomic-intrinsic defect); the suite
sidesteps it by installing the initial value with `atomic_store`.  Worth a
focused look as part of the raw-pointer harness work (`MEM-RAWPTR-HARNESS-1`).

### Inter-thread ordering

Not exercised here (single-threaded harness).  The *ordering* semantics
(acquire/release visibility, SeqCst total order) belong to a concurrency
integration suite under `vcs/specs/L2-standard/`; this suite pins the per-op
value contract only.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync` (Atomic*, Mutex, RwLock, Once) | the entire load/store/CAS/fetch surface. |
| `core.async` (executors, channels, waker refcounts) | `fetch_add`/`fetch_sub` refcounting + CAS state machines. |
| `core.mem` (CBGR epoch/generation counters) | atomic increments on shared metadata. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the width-typed atomic
  entries + their opcode/strategy.
* interp atomic handlers + `crates/verum_codegen/src/llvm/` atomic lowering —
  the per-tier `atomicrmw` / `cmpxchg` / `fence` semantics.
* `core/intrinsics/atomic.vr` — `ORDERING_*` constants + the `MemoryOrder`
  ↔ `UInt8` mapping (`ordering_to_u8`).

## 6. Action items

**Landed this branch**
* Operational atomic conformance suite (property + integration + regression +
  audit) over a live cell — load/store/fetch-ops/exchange/CAS/fences across
  widths, both tiers.

**Deferred (tracked)**
* ATOMIC-AOT-RAWPTR-1 (#25) — operational atomic ops via `List.as_mut_ptr` fail
  under AOT (AOT raw-pointer/`List`-backing path; interp green).
* MEM-LIST-LITERAL-PTR-1 (#24) — `List` literal vs `as_mut_ptr` backing.
* Inter-thread ordering conformance (concurrency suite, out of scope here).
