# `net/h3/frame` audit

Module: `core/net/h3/frame.vr` — RFC 9114 §7.2 frames: FT_* type
codepoints (§11.2.1), SETTING_* parameter ids, `H3FrameError`
3-variant ADT, `H3Frame` 8-variant ADT + stream-legality predicates,
`decode_frame`/`encode_frame` (varint codec).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.h3.connection` | control/request/push stream frame dispatch. |
| `core.net.h3.qpack` | HEADERS field-section payloads. |
| `core.encoding.varint` | frame type/length varints. |

## 2. Crate-side hardcodes

FT_* (DATA=0x00 … MAX_PUSH_ID=0x0D) and SETTING_* (QPACK_MAX_TABLE_CAPACITY
=0x01, MAX_FIELD_SECTION_SIZE=0x06, H3_DATAGRAM=0x33 per RFC 9297) are RFC
9114 §11.2.1 / §7.2.4.1 + RFC 9220/9297 verbatim. The §6.1 stream-legality
split (SETTINGS/GOAWAY/MAX_PUSH_ID/CANCEL_PUSH → control; DATA/HEADERS/
PUSH_PROMISE → request; RESERVED → both) is pinned by the predicate tests.

## 3. Language-implementation findings

None for the covered surface. H3Frame is a mixed tuple/record ADT;
is_control_stream_legal / is_request_stream_legal are tag-only `match`
dispatch. Constructed variants use empty-`List` / scalar payloads (safe).
`decode_frame`/`encode_frame` (varint + sub-slice) gated on EXTSLICE-1.

## 4. Action items landed in this branch

* `unit_test.vr` — 7 FT_* + 5 SETTING_* codepoint pins; H3FrameError ADT
  (Eq + disjoint); 9 stream-legality predicate cases across the variants.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| decode_frame/encode_frame round-trip (varint) | this folder | gated on EXTSLICE-1 |
| H3Frame payload-preservation per variant | this folder | 1h |
