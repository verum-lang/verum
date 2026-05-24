# `core.io.engine` — audit

> Conformance suite for `core/io/engine.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Constants + IoEvent flag
> composition are stable. IoEngine lifecycle (new/destroy/register/poll)
> needs a kernel-resource sandbox.

## 1. Cross-stdlib usage

* `core.net.tcp` / `core.net.udp` — async socket I/O registers fds
  with an IoEngine for readiness notifications.
* `core.io.async_protocols` — Future polling defers to engine.poll
  in the long-term plan.
* `core.async.executor` — wakes tasks when their IoEngine readiness
  signal fires.

## 2. Crate-side hardcodes

* `crates/verum_vbc/src/intrinsics/registry.rs` — `__io_engine_new`,
  `__io_engine_register`, `__io_engine_poll`, `__io_engine_destroy`
  bridge to libc's kqueue/epoll/IOCP per-platform.

## 3. Language-implementation gaps

### §A — Engine lifecycle needs sandboxed test harness (#io-16)

IoEngine.new() allocates a kernel fd (kqueue/epoll/IOCP handle).
Without a sandboxed test environment that guarantees engine handles
are released between tests, parallel test execution can leak fds.

**Tracking task #io-16**: sandboxed IoEngine test harness with
automatic destroy on test exit.

### §B — Real async I/O via engine.poll is still in the plan

The current async I/O implementation in `core.io.async_protocols`
falls back to sync-on-worker-thread (per the in-source comments).
Real engine.poll routing is task #io-15.

## 4. Action items landed

* Created `core-tests/io/engine/` with `unit_test.vr` (constants
  pin: 256/65536/4096 + power-of-2 max + IoEvent flag values 1/2/3),
  `property_test.vr` (constants ordering + power-of-2 invariant +
  Read|Write = ReadWrite law), `integration_test.vr` (event flag
  aggregation), `regression_test.vr` (2 @ignore'd lifecycle pins).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-15 | Real async I/O via io_uring/kqueue/IOCP | 1-2 weeks |
| #io-16 | Sandboxed IoEngine test harness | 1 day |
