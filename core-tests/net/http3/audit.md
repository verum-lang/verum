# `net/http3` audit

Module: `core/net/http3/` (~6 files) — RFC 9114 HTTP/3 protocol
+ RFC 9204 QPACK. Sibling to `core/net/h3/` (newer pure-Verum
implementation; this module is the original).

Tests cover the wire-format constants:

* H3FrameType 7+Reserved variants — to_u64 / from_u64 round-trip
  (DATA 0x0 / HEADERS 0x1 / CANCEL_PUSH 0x3 / SETTINGS 0x4 /
  PUSH_PROMISE 0x5 / GOAWAY 0x7 / MAX_PUSH_ID 0xD per RFC 9114
  §7.2 + §11.2.1).
* H3 SETTINGS parameter IDs (RFC 9114 §7.2.4.1):
  QPACK_MAX_TABLE_CAPACITY 0x01, MAX_FIELD_SECTION_SIZE 0x06,
  QPACK_BLOCKED_STREAMS 0x07.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic` | underlying QUIC transport. |
| `core.net.h3` | sibling pure-Verum impl. |
| `core.encoding.varint` | QUIC varint codec for frame length. |

## 2. Crate-side hardcodes

H3 wire IDs are IANA-registered; pinned by `test_h3_frame_type_*`
+ `test_h3_setting_*`.

## 3. Language-implementation gaps

### §3.1 HTTP3-1 — Frame encode / decode functional surface

Subject to precompile-cascade SIGSEGV class. Wire-constant
data-surface compiles and tests pass.

### §3.2 HTTP3 vs h3 — duplicate implementation

`core.net.http3` and `core.net.h3` are parallel implementations.
The newer pure-Verum `h3` covers QPACK + push + WebTransport;
roadmap is to subsume `http3` into `h3`.

## 4. Action items landed in this branch

* `core-tests/net/http3/unit_test.vr` — 13 unit tests covering
  H3FrameType 7 wire IDs + 3 round-trips + 3 settings param IDs.
* `core-tests/net/http3/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Http3Frame 8-variant payload preservation | this folder | 1h |
| Http3Error variant disjointness | this folder | 1h |
| Frame encode/decode round-trip with varint | this folder | 4h, gated on §3.1 |
| QPACK static-table + dynamic-table coverage | this folder | 1 day |
| Merge http3 + h3 — single canonical impl | stdlib refactor | 1 week |
