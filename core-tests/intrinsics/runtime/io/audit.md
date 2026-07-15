# `intrinsics/runtime/io` audit

Module: `core/intrinsics/runtime/io.vr` (~116 LOC) — I/O engine
(kqueue/epoll/IOCP), socket options, async fd ops.

Tests: unit (5) — engine lifecycle (new/destroy/distinct), zero-timeout
poll on an empty engine, readiness of an unregistered fd.
Property (6, added 2026-07-15) — the fd-free engine ALGEBRA: pairwise
handle distinctness (4 live engines), capacity-domain acceptance
(1/16/256/4096), empty-poll stability across repeats and max-event
values, uniform non-readiness of unregistered fds over an fd domain,
take_ready purity on an empty engine.  These are the cheapest
cross-tier drift probes (interp handler ↔ per-triple AOT lowering).

## Coverage decisions

* Socket-option intrinsics (`__socket_set_*`) and `__async_accept/read/
  write_raw` need LIVE fds — they are exercised end-to-end by the net
  suites (`core-tests/net/*`, TCP paths).  Pinning them here would mean
  opening sockets in the intrinsics tree — wrong home.
* `io_submit`/`io_remove`/`io_modify`/`io_take_ready` are meaningful only
  with a registered fd → same decision.

## Crate-side drift surfaces

* `handlers/io_engine.rs` (interp) ↔ per-triple AOT lowering
  (kqueue/epoll/IOCP) — the empty-engine poll semantics (0 on timeout)
  are the cheapest cross-tier drift probe and are pinned here.

## Action items

* Loopback-socket integration (engine + async_read/write round-trip) in
  the net suite — flag raised, not duplicated here.
