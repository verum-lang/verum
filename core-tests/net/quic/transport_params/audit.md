# `net/quic/transport_params` audit

Module: `core/net/quic/transport_params.vr` — RFC 9000 §18.2 transport
parameters: `TransportParams` record + `PreferredAddress`, public bound
constants, `defaults()` + `bounds_ok()`. The PARAM_* IANA ids (§18.2)
are module-private (`const`, not `public const`).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | negotiated params drive flow control + limits. |
| `core.net.tls13.handshake` | params ride the quic_transport_parameters extension. |

## 2. Crate-side hardcodes

Public bounds (MAX_STREAMS_LIMIT=2^60, MAX_ACK_DELAY_EXPONENT=20,
MIN_ACTIVE_CID_LIMIT=2, MIN_MAX_UDP_PAYLOAD_SIZE=1200) are RFC 9000 §18.2
verbatim and pinned. `defaults()` is asserted to satisfy `bounds_ok()`.

## 3. Language-implementation findings

None for the covered surface. `defaults()` constructs the record and
`bounds_ok()` reads fields — both compile cleanly under `--interp`.

The §18.2 PARAM_* wire ids are private; per-id pinning is deferred until
they are exposed or the encode/decode round-trip is covered (varint +
sub-slice → gated on EXTSLICE-1).

## 4. Action items landed in this branch

* `unit_test.vr` — 4 public bound constants + defaults().bounds_ok().

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| PARAM_* wire-id pins (needs public exposure) | stdlib + tests | 1h |
| Transport-params encode/decode round-trip | this folder | gated on EXTSLICE-1 |
| bounds_ok() rejection cases (out-of-range fields) | this folder | 1h |
