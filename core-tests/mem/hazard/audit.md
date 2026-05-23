# `core.mem.hazard` — audit findings

> Module under test: `core/mem/hazard.vr` (629 LOC; 3 constants
> (HAZARD_POINTERS_PER_THREAD=8, RETIRED_THRESHOLD=64, MAX_THREADS=256),
> records HazardDomain/ThreadRecord/ThreadHazardRecord/RetiredNode/
> HazardGuard/HazardStats, free functions acquire_hazard, retire_hazard,
> force_reclaim_all, hazard_stats, cleanup_thread_hazards).
>
> Test surfaces (this branch):
> `unit_test.vr` (~85 LOC), `property_test.vr` (~40 LOC),
> `integration_test.vr` (~35 LOC), `regression_test.vr` (~40 LOC).
>
> Static-shape only — live hazard acquire / retire round-trip is
> covered in `core-tests/base/memory/cbgr_test.vr`. SPMC race
> properties (concurrent-deref vs concurrent-free safety) require
> task spawning that this branch's test infrastructure does not
> provide.

## 1. Cross-stdlib usage

| Consumer | Use |
|---|---|
| `core/mem/thin_ref.vr` | `deref_thin` acquires a hazard before reading. |
| `core/mem/fat_ref.vr` | Same — every CBGR deref under contention installs a hazard. |
| `core/mem/header.vr` | `try_revoke` scans the global hazard table before freeing. |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `HAZARD_POINTERS_PER_THREAD = 8` | Per-thread slot count | Tuning constant; affects max concurrent-deref depth. |
| `RETIRED_THRESHOLD = 64` | Retired-list size before reclamation | Lower = more frequent scans; higher = more memory pressure. |
| `MAX_THREADS = 256` | Global thread cap | Drift = either OOM on excess threads or under-utilisation. |

## 3. Language-implementation gaps

### 3.1 Concurrent race coverage

The hazard system's correctness invariant — "a reader's hazard
must be installed BEFORE the freer scans" — requires multi-thread
testing that this branch's test infrastructure cannot provide.
SPMC race coverage deferred.

### 3.2 Hazard guard RAII

`HazardGuard` should drop-clear its hazard slot when going out of
scope. Tests pin the construction surface but not the drop
behaviour — drop testing would require observing the global hazard
table before and after a scope exit.

### 3.3 `static mut` record-typed backing (FUNDAMENTAL)

`hazard_stats()` and every method call on `GLOBAL_HAZARD_DOMAIN`
null-deref under `--interp` at `pc=144` of `scan_hazards`, opcode
`0x62` (GetF).  Root cause is architectural: Task #26 [E2] added a
process-wide `Box<UnsafeCell<u64>>` cell allocator
(`InterpreterState::static_mut_cell_addr`) for `static mut`, but the
cell is **scalar-only** (8 bytes, zero-initialised, no offset table)
— record-typed `static mut HazardDomain` has no real backing.  When
the codegen materialises `&self` for an implicit method-receiver
self-load, it produces a Value-encoded null pointer.

This defect class extends past `hazard.vr`:

| Site | Pattern | Affected paths |
|---|---|---|
| `core/mem/hazard.vr:174` | `public static mut GLOBAL_HAZARD_DOMAIN: HazardDomain` | every method on the global, including `hazard_stats()` |
| `core/mem/epoch.vr` (similar pattern) | `static mut GLOBAL_EPOCH: EpochManager` | analogous risk |

Two fix paths:

1. **Codegen + interpreter side**: extend `static_mut_cell_addr`
   to allocate a heap-stable block sized by the record's
   `TypeDescriptor.layout.size` with per-field offsets honoured by
   `compile_static_mut_addr` for field-typed receivers.  Roughly
   2-3 days of VBC work — touches
   `crates/verum_vbc/src/interpreter/state.rs` (cell allocator),
   `crates/verum_vbc/src/codegen/expressions.rs` (field-addr
   detection on static-mut path).
2. **Stdlib side workaround**: refactor `hazard_stats()` and
   `scan_hazards`-callers to read each scalar field via individual
   `&STATIC_MUT.field as *const _` patterns that go through
   `try_compile_static_mut_addr` directly.  This sidesteps the
   record-typed backing gap but spreads boilerplate across every
   `static mut` consumer.

The fundamental fix (path 1) closes the class.  Pinned by the
@ignore'd `test_hazard_stats_returns_value` and
`test_hazard_stats_initial_state_is_zero`.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/hazard.vr` | `core-tests/mem/hazard/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~200 LOC total (static-shape only). |
| 2 | Missing `audit.md` for `core-tests/mem/hazard/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Concurrent SPMC tests — install hazard from reader thread, retire from writer thread, verify reclamation order. | Blocked on task-spawn primitive | open |
| §B | HazardGuard RAII drop test — observe hazard slot before/after scope exit. | ~30 min | open |
| §C | force_reclaim_all behaviour — populate retire list past threshold, observe count drops. | Blocked on live integration | open |
| §D | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
| §E | **FUNDAMENTAL** `static mut` record-typed backing — extend `static_mut_cell_addr` to honour record layout; unblocks `hazard_stats()` and every other record-typed-static-mut consumer (epoch / GlobalAllocator state / cap-audit-ring head pointer if it ever becomes record-shaped). | 2-3 days VBC work | open — see §3.3 |

## 6. Investigation 2026-05-23 — `__tls_init_*` path traced

Read `crates/verum_vbc/src/codegen/mod.rs:6395-6435` and
`interpreter/mod.rs:399-424`.  Findings:

1. **Static-mut record initializers ARE queued** — `is_thread_local
   || static_decl.is_mut` (line 6402-6403) routes EVERY `static mut`
   (including non-thread-local) through `pending_tls_inits`, which
   `compile_pending_tls_inits` (line 10941) lowers to a
   `__tls_init_<NAME>` synthetic function that compiles the init
   expression and `TlsSet`s the result.

2. **Interpreter runs them** — `run_global_ctors` (line 399)
   executes any ctor whose name starts with `__tls_init_*`, regardless
   of the underlying static being thread-local or process-wide.

3. **The bug must be downstream** — either:
   - (a) the init expression `HazardDomain { ... }` doesn't compile
     into a real record Value (some sub-expression like the
     `0 as &unsafe ThreadRecord` cast fails silently);
   - (b) `TlsGet` returns the record Value but the method-receiver
     materialisation produces a null `&self` because the Value is
     by-value and the codegen for `&self.field` (GetF) expects a
     stable memory address that an in-register Value doesn't have.

4. **Cell allocator is NOT in the path** — `static_mut_cell_addr`
   (state.rs:2864) is invoked only by `StaticMutAddr` (0x52), which is
   emitted only for `&IDENT as *T` casts (codegen
   `try_compile_static_mut_addr`).  Method calls on a static mut
   record go through `TlsGet` + method dispatch, not through the
   cell allocator.  So the cell allocator's 8-byte limit is NOT the
   bottleneck for `hazard_stats()` — the bottleneck is the
   `&self.field` lowering on a by-value record self-receiver.

**Proper fix candidates:**

- **Path 1 (interpreter-side):** when `TlsGet` returns a record Value
  for static mut, materialise the record into a process-lifetime
  heap-stable allocation (similar to the task #18 escape-cell
  pattern), and have method-receiver materialisation produce a real
  `ThinRef`/`FatRef` to that allocation.  `&self.field` then resolves
  to a real field address.
- **Path 2 (codegen-side):** when the method receiver is a static
  mut Path, lower the method call to (a) `StaticMutAddr` to get the
  cell base, (b) cast to `&RecordType`, (c) pass as `&self`.  This
  requires the cell allocator to size cells by record layout.

Path 1 is structurally simpler — it piggybacks on the existing
escape-cell infrastructure landed for task #18.  Path 2 is more
efficient but requires record-aware cell allocation.

Tracked as task #10/§3.3.  Investigation logged in
[[task_static_mut_record_typed_backing_2026-05-22]].
