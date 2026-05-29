# `net/http2/frame` audit

Module: `core/net/http2/frame.vr` (~476 LOC) — RFC 7540 §4 frame
codec: `FrameType` (10 + Unknown), `FrameFlags` (bitmap),
`FrameHeader` (the 9-byte big-endian prefix, encode/decode),
`Http2Frame` (11-variant typed payload), `PriorityData`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.mod` | connection-level frame demux. |
| `core.net.http2.hpack` | HEADERS/CONTINUATION block fragments. |
| `core.net.http2.error` | decode failures → `Http2Error`. |

## 2. Crate-side hardcodes

`FRAME_HEADER_SIZE=9`, `MAX_FRAME_SIZE_INITIAL=16384`,
`FRAME_LENGTH_LIMIT=0xFFFFFF` (24-bit), and all 10 FrameType wire
IDs are RFC §4.1 constants. The `property_h2frame_type_round_trip_
all_bytes` law verifies from_u8 ∘ to_u8 == id over the full 0..255
space (Unknown carries the raw byte).

## 3. Language-implementation findings

### §3.1 HTTP2-FRAME-PAYLOAD — `decode_payload` sub-slice paths deferred

`Http2Frame.decode_payload` and its `decode_data` / `decode_headers`
/ … helpers slice the payload buffer with sub-range expressions
(`&payload[start..end]`) — the catalogued EXTSLICE-1 surface. Until
the EXTSLICE-1 compiler-layer fix lands (or those helpers are
rewritten to indexed byte-copy), per-variant payload round-trip is
deferred. The `FrameHeader.encode`/`decode` path (push + indexed
reads only, no sub-range) compiles cleanly and is fully covered
here, including the reserved-bit masking invariant (§4.1 R bit).

## 4. Action items landed in this branch

* `unit_test.vr` — 21 tests: constants; FrameType wire mapping;
  FrameFlags has/with/without/none/raw + 5 constants; FrameHeader
  encode (9-byte, big-endian length, reserved-bit clear); header
  round-trip incl. reserved-bit stripping; short-buffer NeedMore.
* `property_test.vr` — 2 laws: FrameType round-trip over all 256
  bytes; FrameHeader round-trip over length × stream-id sweep.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Http2Frame per-variant decode_payload round-trip | this folder | gated on EXTSLICE-1 (catalogue §1) |
| PriorityData §5.3.2 weight+1 semantics | this folder | 1h |
| SETTINGS frame param list (UInt16,UInt32) codec | this folder | gated on EXTSLICE-1 |
