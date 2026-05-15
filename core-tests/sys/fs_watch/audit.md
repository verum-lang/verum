# `core.sys.fs_watch` — implementation audit

## Status: **partial** (type-shape surface complete; kernel-event happy-path deferred)

* `FsEventKind` 5-variant ADT pinned end-to-end + Clone impl exercised.
* `FsEvent` record construction + field round-trip pinned.
* The **`FsWatcher.new()` / `.watch()` / event-stream** surface is
  **deferred** — it requires a temp-directory fixture + a
  backgrounded writer process to actually emit a kernel event. The
  kernel-event path itself goes through kqueue / inotify /
  ReadDirectoryChangesW per-platform plumbing and is exercised in
  `core-tests/integration/fs_watch/` once that fixture lands.

## 1. Cross-stdlib usage

`core.sys.fs_watch` is the canonical filesystem-watch surface.
Consumers — `core.io.fs`, IDE live-reload, build-system file-watcher
integration.

## 2. Action items landed in this branch

1. `unit_test.vr` — 7 `@test`s.
2. `property_test.vr` — 3 algebraic-law `@test`s.
3. `regression_test.vr` — 2 `@test`s pinning the variant-constructor
   path.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `FsWatcher.new()` happy path | Requires per-platform fixture. |
| 2 | Event-stream round-trip | Needs writer-process pair fixture. |
