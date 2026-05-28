# `net/http3` audit

Module: `core/net/http3/` (~6 files) — RFC 9114 HTTP/3 protocol
+ RFC 9204 QPACK. Sibling to `core/net/h3/` (newer pure-Verum
implementation; this module is the original).

Tests cover the wire-format constants + algebraic data surface:

* H3FrameType 7+Reserved variants — to_u64 / from_u64 round-trip
  (DATA 0x0 / HEADERS 0x1 / CANCEL_PUSH 0x3 / SETTINGS 0x4 /
  PUSH_PROMISE 0x5 / GOAWAY 0x7 / MAX_PUSH_ID 0xD per RFC 9114
  §7.2 + §11.2.1).
* H3 SETTINGS parameter IDs (RFC 9114 §7.2.4.1):
  QPACK_MAX_TABLE_CAPACITY 0x01, MAX_FIELD_SECTION_SIZE 0x06,
  QPACK_BLOCKED_STREAMS 0x07.
* Http3Frame 8-variant disjointness (DataFrame / HeadersFrame /
  SettingsFrame / CancelPushFrame / PushPromiseFrame /
  GoAwayFrame / MaxPushIdFrame / UnknownFrame).
* Http3Frame field-independence (stream_id / push_id / type_
  preserved post-construction).
* Http3Error 6-variant Eq reflexivity + payload preservation
  (FrameError + QpackError carry Text payload).
* Http3ErrorCode wire-stable constants — H3_NO_ERROR 0x100,
  H3_INTERNAL_ERROR 0x102, QPACK_DECOMPRESSION_FAILED 0x200.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic` | underlying QUIC transport. |
| `core.net.h3` | sibling pure-Verum impl. |
| `core.encoding.varint` | QUIC varint codec for frame length. |

## 2. Crate-side hardcodes

H3 wire IDs are IANA-registered; pinned by `test_h3_frame_type_*`
+ `test_h3_setting_*` (unit) and `regression_http3_*` (regression).
Drift in any of these would break HTTP/3 interoperability with
every IANA-conformant peer.

## 3. Language-implementation gaps

### §3.1 HTTP3-1 — Frame encode / decode functional surface

Subject to precompile-cascade SIGSEGV class. Wire-constant
data-surface compiles and tests pass. Functional surface (frame
encode/decode, varint round-trip, QPACK encoder/decoder,
connection state machine) gated on harness landing.

### §3.2 HTTP3-2 — HTTP3 vs h3 — duplicate implementation

`core.net.http3` and `core.net.h3` are parallel implementations.
The newer pure-Verum `h3` covers QPACK + push + WebTransport;
roadmap is to subsume `http3` into `h3`. Test surface here is
maintained alongside `h3` until the merge lands.

### §3.3 QPACK static + dynamic table coverage

RFC 9204 §3.1 defines a 99-entry static table; the dynamic
table is built per-connection. Encoder/decoder behaviour not
yet covered at this layer — deferred to L2 specs.

## 4. Action items landed in this branch

* `core-tests/net/http3/unit_test.vr` — 13 unit tests covering
  H3FrameType 7 wire IDs + 3 round-trips + 3 settings param IDs.
* `core-tests/net/http3/property_test.vr` — 33 property tests
  spanning H3FrameType variant + wire-ID algebra §A-D,
  SETTINGS disjointness §E, Http3Frame variant + field algebra
  §F-G, Http3Error Eq §H, Http3ErrorCode pins §I, wire-ID
  canonical-range stability §J.
* `core-tests/net/http3/regression_test.vr` — 15 regression
  pins (10 active wire-format LOCK-IN, 5 `@ignore`'d functional
  pins for codec round-trip + QPACK + state machine).
* `core-tests/net/http3/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Http3Frame 8-variant payload preservation full coverage | this folder | 1h |
| Http3Error 6-variant disjointness exhaustive matrix | this folder | 1h |
| Frame encode/decode round-trip with varint codec | this folder | 4h, gated on §3.1 |
| QPACK static-table + dynamic-table coverage | this folder | 1 day |
| Merge http3 + h3 — single canonical impl | stdlib refactor | 1 week |
