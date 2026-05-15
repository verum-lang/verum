# `core.sys.io_engine` — implementation audit

## Status: **partial** (value-type surface complete; IOEngine protocol round-trip deferred)

* `EngineDuration` ring algebra (from/as scaling, saturating
  arithmetic, identity laws) is pinned end-to-end via `unit_test.vr`
  + `property_test.vr`.
* `Fd` + `Fd.INVALID` + `Fd.is_valid` + `Fd.try_as_valid`
  partitioning pinned with the full negative ↔ non-negative sweep.
* `TimeSpec` record round-trip pinned.
* The **IOEngine protocol** (`create_io_engine` factory,
  `CompletionOp` 18-variant submission queue, `CompletionResult`,
  per-platform driver round-trip) is **deferred** — the protocol
  requires kernel resources (io_uring fd / kqueue fd / IOCP handle)
  and a per-platform fixture for cross-tier validation. Tracked in
  `core-tests/integration/io_engine/` once the fixture lands.

## 1. Cross-stdlib usage

`core.sys.io_engine` is the canonical IOEngine protocol. Consumers
— every async I/O path in `core.async`, `core.net`, `core.io`.

## 2. Action items landed in this branch

1. `unit_test.vr` — 17 `@test`s covering EngineDuration arithmetic
   + Fd partitioning + TimeSpec shape.
2. `property_test.vr` — 7 algebraic-law `@test`s pinning the
   identity / commutativity / round-trip contracts.
3. `regression_test.vr` — 4 `@test`s pinning the UInt64 / Int32
   width preservation classes.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | IOEngine protocol round-trip | Requires per-platform kernel fixture. |
| 2 | `CompletionOp` 18-variant exhaustive construction | Each variant needs a typed buf/iovec/addr fixture. |
| 3 | `Port` / `BoundPort` refinement compile-time validation | Refinement validator is covered in `crates/verum_smt/` unit tests. |
| 4 | `RawSocketAddr.V4` / `V6` round-trip | Belongs in the integration layer once `addr.port()` method dispatch settles. |
