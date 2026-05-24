# `core.io.async_protocols` — audit

> Conformance suite for `core/io/async_protocols.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Future record construction
> + factory function surface (read_async / write_async / flush_async)
> are stable. Polling the futures requires both the async executor
> runtime AND closing #io-1.

## 1. Cross-stdlib usage

* `core.io.buffer.BufReader.read_line_async` — wraps the sync
  `read_line` in a future; reverts to sync-under-async (matches
  `AsyncFile.read` pattern).
* `core.async.executor` — Future polling primitives live here.
* `core.net.*` — async socket I/O implements AsyncRead / AsyncWrite.

## 2. Crate-side hardcodes

* The Future record layouts (ReadFuture { reader, buf, _polled }
  and similar) are visible at the AST level; the executor polls
  them via a struct-shape-aware dispatch.
* `crates/verum_vbc/src/intrinsics/registry.rs` — `__async_poll`
  and friends route to the executor.

## 3. Language-implementation gaps

### §A — Sync-under-async fallback

Per the doc-comments in this module, the async I/O variants currently
run sync I/O on a worker thread rather than dispatching through
io_uring / kqueue / IOCP. Future work routes through `core.io.engine.IoEngine`.

**Tracking task #io-15** (already known, scoped under engine.vr):
real async I/O via per-platform native multiplexer.

### §B — Awaiting futures gated by #io-1 + executor runtime

The Future body's poll_read / poll_write methods call
`self.inner.read(...)` / `write(...)` which trigger the bare-name
collision. Plus the test would need an executor to actually drive
the future to completion.

## 4. Action items landed

* Created `core-tests/io/async_protocols/` with construction tests for
  ReadFuture / WriteFuture / FlushFuture, factory function tests
  (read_async / write_async / flush_async), exhaustive-buffer-size
  property tests.

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function | 3-5 days |
| #io-15 | Real async I/O via io_uring/kqueue/IOCP | 1-2 weeks (scoped under engine.vr) |
