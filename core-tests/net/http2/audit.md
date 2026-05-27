# `net/http2` audit

Module: `core/net/http2/` (~1400 LOC across 8 files) — RFC 7540
HTTP/2 protocol + RFC 7541 HPACK codec.

Tests cover the wire-format constant surface — the canonical
IANA-registered identifiers that govern frame parsing and
settings negotiation. The IANA wire values are RFC-stable;
drift here would break HTTP/2 interoperability silently.

Coverage:

* FRAME_HEADER_SIZE = 9, MAX_FRAME_SIZE_INITIAL = 16384,
  FRAME_LENGTH_LIMIT = 0xFFFFFF (24-bit).
* PREFACE 24-byte "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" connection
  preface (RFC 7540 §3.5).
* FrameType 10-variant + Unknown — to_u8 wire IDs 0x0-0x9
  (DATA / HEADERS / PRIORITY / RST_STREAM / SETTINGS /
  PUSH_PROMISE / PING / GOAWAY / WINDOW_UPDATE / CONTINUATION)
  + from_u8 round-trip + unknown fall-through.
* FrameFlags 5 well-known constants (END_STREAM 0x01,
  END_HEADERS 0x04, PADDED 0x08, PRIORITY 0x20, ACK 0x01) +
  has / with / without bitmap composition.
* SettingId 6 well-known parameter IDs (HEADER_TABLE_SIZE 0x1
  ... MAX_HEADER_LIST_SIZE 0x6) per RFC 7540 §6.5.2.
* Settings defaults per §6.5.2 initial values.
* MAX_INITIAL_WINDOW_SIZE 2^31-1, MIN_MAX_FRAME_SIZE 16384,
  MAX_MAX_FRAME_SIZE 2^24-1 bounds.
* ErrorCode 14 RFC 7540 §7 wire constants (NO_ERROR 0x00 ...
  HTTP_1_1_REQUIRED 0x0D).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` HTTP/2 server | frame loop + HPACK decode. |
| `core.net.http3` parameter inheritance | HTTP/3 inherits Method / StatusCode / Headers shape. |
| `core.net.tls` ALPN | "h2" protocol selection. |

## 2. Crate-side hardcodes

Every wire ID (FrameType to_u8 / FrameFlags constants / SettingId
IDs / ErrorCode constants) is RFC-stable; tests pin them
individually. Drift here would produce non-interoperable HTTP/2
endpoints — pinned by 40+ explicit value-equality tests.

## 3. Language-implementation gaps

### §3.1 HTTP2-1 — HPACK encoder/decoder + stream state machine

Same precompile-cascade SIGSEGV class as CIDR-1 family. The
data-surface (wire constants + Settings record) compiles and
tests pass. Functional surface (`HpackEncoder.encode`,
`HpackDecoder.decode`, `StreamFsm.transition`, `Settings.apply`)
covered at L2 specs.

### §3.2 RFC 7540 vs RFC 9113 — HTTP/2 spec revision

RFC 9113 (June 2022) obsoleted RFC 7540 with editorial +
some semantic changes (deprecated stream prioritisation §5.3,
SETTINGS_NO_RFC7540_PRIORITIES parameter 0x9). The module is
labeled RFC 7540; SETTINGS_NO_RFC7540_PRIORITIES is not
exported — tracked as roadmap.

### §3.3 Stream prioritisation deprecated

Per RFC 9113 §5.3.4, stream prioritisation is deprecated. The
PRIORITY frame still parses but is advisory.

### §3.4 HPACK Huffman + static-table tests

Per RFC 7541 §A static table has 61 fixed entries; §B has the
canonical Huffman code table. Both are intrinsic-stable tables
— pinning their layout requires reading the implementation
files; deferred to follow-up.

## 4. Action items landed in this branch

* `core-tests/net/http2/unit_test.vr` — 49 unit tests covering
  wire-format constants (5: FRAME_HEADER_SIZE + MAX_FRAME_SIZE_INITIAL
  + FRAME_LENGTH_LIMIT + PREFACE.len + PRI bytes), FrameType
  10 wire IDs + 3 from_u8 round-trips, FrameFlags 5 constants
  + 4 bitmap ops, SettingId 6 IDs, Settings 6 defaults + 3
  bounds + record shape, ErrorCode 14 RFC 7540 §7 constants.
* `core-tests/net/http2/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| HPACK static-table coverage (RFC 7541 §A 61 entries) | this folder | 2h |
| HPACK Huffman canonical code table (RFC 7541 §B) | this folder | 2h |
| StreamState 6-variant + transition matrix (§5.1) | this folder | 4h |
| Settings.apply RFC-bound enforcement (PROTOCOL_ERROR / FLOW_CONTROL_ERROR) | this folder | 2h, gated on §3.1 |
| FrameHeader.encode / decode round-trip | this folder | 2h, gated on §3.1 |
| RFC 9113 NO_RFC7540_PRIORITIES + Extended Connect support | stdlib + tests | 1 day |
| End-to-end HTTP/2 frame loop with HPACK round-trip | language level | 1 week |
