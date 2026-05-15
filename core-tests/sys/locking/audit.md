# `core.sys.locking` — implementation audit

## Status: **partial** (value-type surface complete; live-lock round-trip deferred)

* `FileLockKind` 2-variant ADT pinned end-to-end.
* `LockRegion` start + length record round-trip pinned with the full
  boundary sweep (-1 EOF sentinel, 0, positive).
* `LockError.Conflict(Maybe<Int>)` + `LockError.IoError(OSError)`
  variant construction pinned at the user-side match destructure
  layer.
* The **`try_lock` / `unlock` live round-trip** is **deferred** —
  requires a real fd + second-holder fixture. Tracked in
  `core-tests/integration/locking/`.

## 1. Cross-stdlib usage

`core.sys.locking` is the byte-range-advisory-lock surface used
primarily by `core.database.sqlite.native.l0_vfs.locking.vr` (the
5-state SQLite locking protocol is built on top).

## 2. Action items landed in this branch

1. `unit_test.vr` — 7 `@test`s covering FileLockKind / LockRegion /
   LockError construction.
2. `property_test.vr` — 3 algebraic-law `@test`s pinning field
   round-trip and 2-variant disjointness.
3. `regression_test.vr` — 3 `@test`s pinning the FileLockKind rename
   (task #160) and the LockError payload shape.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `try_lock` / `unlock` happy path | Requires fd fixture. |
| 2 | Cross-process conflict detection | Requires fixture pair of processes. |
| 3 | `LockError.eq` exhaustive Eq laws | Requires sibling `OSError` fixture + Maybe<Int> sweep. |
