# `net/h3` audit

Module: `core/net/h3/` (~13 files) — pure-Verum HTTP/3
(RFC 9114) + QPACK (RFC 9204) + RFC 9220 Extended CONNECT +
RFC 9297 H3 Datagram + WebTransport (W3C). The newer
implementation that will subsume `core/net/http3/`.

Tests cover the wire-format constants:

* 7 H3 frame type IDs per RFC 9114 §11.2.1 (FT_DATA 0x00,
  FT_HEADERS 0x01, FT_CANCEL_PUSH 0x03, FT_SETTINGS 0x04,
  FT_PUSH_PROMISE 0x05, FT_GOAWAY 0x07, FT_MAX_PUSH_ID 0x0D).
* 5 SETTINGS parameter IDs (QPACK_MAX_TABLE_CAPACITY 0x01,
  MAX_FIELD_SECTION_SIZE 0x06, QPACK_BLOCKED_STREAMS 0x07,
  ENABLE_CONNECT_PROTOCOL 0x08 per RFC 9220, H3_DATAGRAM 0x33
  per RFC 9297).
* 4 Stream-type prefix bytes per RFC 9114 §6.2.2 (Control,
  Push, QPACK Encoder, QPACK Decoder) + pairwise-distinctness.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic` | underlying QUIC transport (RFC 9000). |
| `core.net.http3` | sibling implementation; h3 is the future canonical impl. |
| WebTransport | uses H3 Datagram setting. |

## 2. Crate-side hardcodes

Every H3 wire ID is IANA-registered per RFC 9114 §11.2.1. Drift
would break HTTP/3 interoperability silently. Pinned by 16
explicit value-equality tests.

## 3. Language-implementation gaps

### §3.1 H3-1 — frame encode/decode + connection state machine

Same precompile-cascade SIGSEGV class as CIDR-1 family. Wire-
constant data-surface compiles and tests pass. Functional
surface (`decode_frame` / `encode_frame` / connection.vr state
machine / request.vr + response.vr) gated on L2 specs.

### §3.2 H3 vs http3 duplication

Two parallel implementations of HTTP/3. Roadmap is to subsume
`http3` into `h3`. Tracking via internal/specs/tls-quic.md.

### §3.3 Extended CONNECT (RFC 9220) — WebSockets over HTTP/3

`SETTING_ENABLE_CONNECT_PROTOCOL = 0x08` pinned by
`test_setting_enable_connect_protocol_0x08`. Server-side
handshake gated on websocket.vr integration.

### §3.4 H3 Datagram (RFC 9297) — for WebTransport

`SETTING_H3_DATAGRAM = 0x33` pinned by
`test_setting_h3_datagram_0x33`. Datagram-frame coverage at L2
specs.

## 4. Action items landed in this branch

* `core-tests/net/h3/unit_test.vr` — 17 unit tests covering 7
  frame type IDs + 5 settings parameter IDs + 4 stream-type
  prefix bytes + 1 pairwise-distinct lattice.
* `core-tests/net/h3/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| H3Frame 8-variant payload preservation | this folder | 1h |
| H3FrameError disjointness | this folder | 1h |
| QPACK static-table coverage (RFC 9204 §3.1) | this folder | 2h |
| Encode/decode round-trip per frame variant | this folder | 4h, gated on §3.1 |
| Extended CONNECT WebSocket-over-H3 server-side | stdlib + tests | 1 week |
| WebTransport datagram + stream coverage | stdlib + tests | 1 week |
| Merge http3 + h3 — single canonical impl | stdlib refactor | 2 weeks |
