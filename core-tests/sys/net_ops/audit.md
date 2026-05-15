# `core.sys.net_ops` — implementation audit

## Status: **partial** (raw-intrinsic mount migration landed; live-socket round-trip deferred)

* Every Raw* newtype (`RawTcpStream`, `RawTcpListener`, `RawUdpSocket`)
  has its `fd` field round-trip pinned at the construction layer.
* The Maybe-shape contract of `connect` / `bind` is pinned via the
  unroutable-port negative-case sweep.
* The `Raw*` canonical-name pin (closed #75 — the rich API at
  `core.net.tcp` / `core.net.udp` no longer shadows the FFI-only
  fallback) is pinned in `regression_test.vr` §B.
* The **live-socket round-trip** (bind + accept loop, send / recv
  byte echo) is **deferred** — it requires a fixture pair of
  processes the test harness can run concurrently. Tracked in
  `core-tests/async/parallel/audit.md` for the right tier.

## 1. Cross-stdlib usage

`core.sys.net_ops` is the raw FFI-only fallback. Consumers — the
rich API at `core.net.tcp` / `core.net.udp` uses the same underlying
intrinsics but goes through the IoEngine for async + CBGR layering.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs` | `__tcp_*_raw` / `__udp_*_raw` net-runtime dispatch | OK (goes through `std::net`) |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Stale `super.raw.*` mount (CLOSED in this branch)

* Same architectural defect as the one closed for `time_ops.vr`,
  `context_ops.vr`, `file_ops.vr`, `process_ops.vr`.
* **Status**: **CLOSED** in this branch.

### 3.2 Rich-API shadow (CLOSED #75)

* Pinned by `regression_test.vr` §B.

## 4. Action items landed in this branch

1. **Fundamental fix**: replaced `mount super.raw.*` in
   `core/sys/net_ops.vr` with the canonical
   `mount core.intrinsics.runtime.os.{__tcp_*_raw, __udp_*_raw}`.
2. `unit_test.vr` — 6 `@test`s.
3. `property_test.vr` — 4 algebraic-law `@test`s.
4. `integration_test.vr` — 2 `@test`s.
5. `regression_test.vr` — 6 `@test`s.

## 5. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live socket round-trip | Requires fixture process pair. |
| 2 | `RawUdpSocket.send_to` host/port shape pin | Deferred until live-socket round-trip lands. |
