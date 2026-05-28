# `net/h3` audit

Module: `core/net/h3/` (~13 files) — pure-Verum HTTP/3
(RFC 9114) + QPACK (RFC 9204) + RFC 9220 Extended CONNECT +
RFC 9297 H3 Datagram + WebTransport (W3C). The newer
implementation that will subsume `core/net/http3/`.

Tests cover the wire-format constant surface + algebraic data
surface (H3Frame variants + payload preservation + stream
legality dispatch + H3FrameError disjointness + UniStreamType
variants + H3ErrorCode RFC 9114 §8.1 constants).

* 7 H3 frame type IDs per RFC 9114 §11.2.1 (FT_DATA 0x00,
  FT_HEADERS 0x01, FT_CANCEL_PUSH 0x03, FT_SETTINGS 0x04,
  FT_PUSH_PROMISE 0x05, FT_GOAWAY 0x07, FT_MAX_PUSH_ID 0x0D).
* 5 SETTINGS parameter IDs (QPACK_MAX_TABLE_CAPACITY 0x01,
  MAX_FIELD_SECTION_SIZE 0x06, QPACK_BLOCKED_STREAMS 0x07,
  ENABLE_CONNECT_PROTOCOL 0x08 per RFC 9220, H3_DATAGRAM 0x33
  per RFC 9297).
* 4 Stream-type prefix bytes per RFC 9114 §6.2.2 (Control,
  Push, QPACK Encoder, QPACK Decoder) + pairwise-distinctness.
* H3ErrorCode wire-stable constants (H3_NO_ERROR 0x0100,
  H3_GENERAL_PROTOCOL_ERROR 0x0101, QPACK_DECOMPRESSION_FAILED
  0x0200).
* H3Frame variant disjointness + payload preservation
  (CancelPushFrame / GoawayFrame / MaxPushIdFrame /
  PushPromiseFrame / ReservedFrame).
* H3Frame stream-legality dispatch (RFC 9114 §6.2.1:
  is_control_stream_legal / is_request_stream_legal).
* H3FrameError 3-variant Eq + InvalidValue payload identity.
* UniStreamType 5-variant + PushStream / ReservedStream payload
  preservation.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic` | underlying QUIC transport (RFC 9000). |
| `core.net.http3` | sibling implementation; h3 is the future canonical impl. |
| WebTransport | uses H3 Datagram setting. |
| `core.encoding.varint` | QUIC varint codec for frame length. |

## 2. Crate-side hardcodes

Every H3 wire ID is IANA-registered per RFC 9114 §11.2.1. Drift
would break HTTP/3 interoperability silently. Pinned by 16
explicit value-equality tests + 9 active regression LOCK-IN
pins.

## 3. Language-implementation gaps

### §3.1 H3-1 — frame encode/decode + connection state machine

Same precompile-cascade SIGSEGV class as CIDR-1 family. Wire-
constant data-surface compiles and tests pass. Functional
surface (`decode_frame` / `encode_frame` / `decode_stream_type` /
connection.vr state machine / request.vr + response.vr) gated
on L2 specs.

### §3.2 H3-2 — H3 vs http3 duplication

Two parallel implementations of HTTP/3. Roadmap is to subsume
`http3` into `h3`. Tracking via internal/specs/tls-quic.md.

### §3.3 Extended CONNECT (RFC 9220) — WebSockets over HTTP/3

`SETTING_ENABLE_CONNECT_PROTOCOL = 0x08` pinned by
`test_setting_enable_connect_protocol_0x08` (unit) and
`regression_h3_setting_enable_connect_pinned_0x08`
(regression). Server-side handshake gated on websocket.vr
integration.

### §3.4 H3 Datagram (RFC 9297) — for WebTransport

`SETTING_H3_DATAGRAM = 0x33` pinned by
`test_setting_h3_datagram_0x33` (unit) and
`regression_h3_setting_h3_datagram_pinned_0x33` (regression).
Datagram-frame coverage at L2 specs.

## 4. Action items landed in this branch

* `core-tests/net/h3/unit_test.vr` — 17 unit tests covering 7
  frame type IDs + 5 settings parameter IDs + 4 stream-type
  prefix bytes + 1 pairwise-distinct lattice.
* `core-tests/net/h3/property_test.vr` — 32 property tests
  spanning frame-type/setting/stream-type disjointness §A-D,
  H3Frame variant + payload algebra §E-F, stream-legality
  dispatch §G, H3FrameError Eq §H, UniStreamType §I,
  H3ErrorCode wire pins §J.
* `core-tests/net/h3/regression_test.vr` — 14 regression pins
  (9 active wire-format LOCK-IN, 5 `@ignore`'d functional pins
  for encode/decode + QPACK + state machine).
* `core-tests/net/h3/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| H3Frame full-coverage encode/decode round-trip per variant | this folder | 4h, gated on §3.1 |
| QPACK static-table coverage (RFC 9204 §3.1 — 99 entries) | this folder | 2h |
| QPACK dynamic-table insertion + reference round-trip | this folder | 4h, gated on §3.1 |
| H3Error 12-variant disjointness (DispatchError / FrameDecode / etc.) | this folder | 1h |
| Extended CONNECT WebSocket-over-H3 server-side | stdlib + tests | 1 week |
| WebTransport datagram + stream coverage | stdlib + tests | 1 week |
| Merge http3 + h3 — single canonical impl | stdlib refactor | 2 weeks |
