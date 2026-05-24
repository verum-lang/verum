# `core.io.buffer` — audit

> Conformance suite for `core/io/buffer.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Constants + construction
> surface for BufReader / BufWriter / BufferCursor / FdReader are
> stable. Method-call surface (read / write / seek / flush / line-based
> reads) is gated by task #io-1.

## 1. Cross-stdlib usage

* `core.shell.stream` — uses `BufReader<FdReader>` to stream stdout
  from spawned processes.
* `core.io.file.{File}` — uses `BufReader` / `BufWriter` wrappers as the
  default buffered I/O path.
* `core.io.stdio.StdinLock` — has its own buffered surface mirroring
  `BufRead` but doesn't share the BufReader implementation; pin the
  contract drift here.

## 2. Crate-side hardcodes

* `crates/verum_vbc/src/intrinsics/registry.rs` — registers `__fd_read_chunk`
  and `__fd_close` intrinsics that `FdReader` bridges to. No data-shape
  drift surface — the intrinsics are bare-fn references.

## 3. Language-implementation gaps

### §A — Cursor<T> naming collision with protocols.vr

See `core-tests/io/protocols/audit.md` §1. The `Cursor<T>` type alias
in this module (line 660) shadows the protocols.vr Cursor<T> definition.
Mount order determines which wins at runtime. Tracking task **#io-2**.

### §B — BufReader / BufWriter / BufferCursor method-call surface gated by #io-1

Every method that chains through `self.inner.read(...)` / `write(...)` /
`seek(...)` hits the bare-name shadow class (same as `core.io.protocols`
audit §A). All read/write/seek tests in `regression_test.vr` are
`@ignore`'d until **#io-1** closes.

## 4. Action items landed

* **Created** `core-tests/io/buffer/` with `unit_test.vr` (constants +
  construction), `property_test.vr` (capacity-clamp law, position
  round-trip, power-of-2 constants), `integration_test.vr` (composition
  with List<Byte> / FdReader), `regression_test.vr` (7 `@ignore`'d pins).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function (covers buffer too) | 3-5 days |
| #io-2 | Deduplicate Cursor<T> between protocols.vr and buffer.vr | 0.5 day after #io-1 |
