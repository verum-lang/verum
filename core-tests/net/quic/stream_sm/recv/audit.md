# `net/quic/stream_sm/recv` audit

Module: `core/net/quic/stream_sm/recv.vr` — RFC 9000 §3.2 receive-stream
state machine: `RecvState` 6-variant + `RecvStream` (`&mut self`) +
`QuicRecvError`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.stream_sm.flow_control` | per-stream receive flow control. |
| `core.net.quic.connection_sm` | STREAM/RESET_STREAM frame ingestion. |

## 2. Crate-side hardcodes

The 6 RFC 9000 §3.2 receive states (Recv → SizeKnown → DataRecvd → DataRead;
ResetRecvd → ResetRead) are pinned by variant + disjointness tests.

## 3. Language-implementation findings

None for the RecvState enum surface (unit variants, `is`-disjoint). The
`RecvStream` ingestion machine (`&mut self` + reassembly + `(bytes, is_fin)`
returns) is gated on the bind-event discipline + Duration/slice surfaces.

## 4. Action items landed in this branch

* `unit_test.vr` — 6 RecvState variants + 3 disjointness cases (data vs
  reset path, terminal vs initial).

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| RecvStream.receive ordering + fin handling (bind-event) | this folder | gated |
| QuicRecvError ADT coverage | this folder | 1h |
| RecvState transition legality matrix | this folder | gated on FSM |
