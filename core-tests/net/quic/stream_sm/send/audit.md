# `net/quic/stream_sm/send` audit

Module: `core/net/quic/stream_sm/send.vr` — RFC 9000 §3.1 send-stream
state machine: `SendState` 6-variant + `SendStream` (`&mut self`) +
`QuicSendError`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.stream_sm.flow_control` | per-stream send flow control. |
| `core.net.quic.connection_sm` | STREAM/RESET_STREAM frame emission. |

## 2. Crate-side hardcodes

The 6 RFC 9000 §3.1 send states (Ready → Send → DataSent → DataRecvd;
ResetSent → ResetRecvd) are pinned by variant + disjointness tests.

## 3. Language-implementation findings

None for the SendState enum surface (unit variants, `is`-disjoint). The
`SendStream` machine (`&mut self`, write/fin/reset) is gated on the
bind-event discipline.

## 4. Action items landed in this branch

* `unit_test.vr` — 6 SendState variants + 3 disjointness cases.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| SendStream.write/finish/reset lifecycle (bind-event) | this folder | gated |
| QuicSendError ADT coverage | this folder | 1h |
| SendState transition legality matrix | this folder | gated on FSM |
