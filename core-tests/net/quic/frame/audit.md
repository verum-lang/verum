# `net/quic/frame` audit

Module: `core/net/quic/frame.vr` (~600 LOC) — QUIC frame ADT
(RFC 9000 §19) + attribute predicates (§13.2.1 ack-eliciting,
§9.1 probing, flow-control) + `decode_frame` / `encode_frame`
(varint-based wire codec) + `FrameError`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | rx/tx frame processing. |
| `core.net.quic.recovery` | ack-eliciting drives loss detection. |
| `core.net.quic.stream_sm` | STREAM frame flow control. |
| `core.encoding.varint` | frame length/field varints. |

## 2. Crate-side hardcodes

The §13.2.1 non-ack-eliciting set (PADDING, ACK, CONNECTION_CLOSE)
and the §9.1 probing set (PADDING, NEW_CONNECTION_ID,
PATH_CHALLENGE, PATH_RESPONSE) are RFC-fixed and pinned by the
predicate partition tests.

## 3. Language-implementation findings

### §3.1 QUIC-FRAME-CODEC — decode_frame / encode_frame sub-slice paths deferred

`decode_frame` and the `decode_*` helpers slice the input buffer
with sub-range expressions and use the varint codec — the EXTSLICE-1
surface. Per-frame encode/decode round-trip is deferred until the
EXTSLICE-1 compiler-layer fix lands (or those helpers move to
indexed byte-copy). The pure attribute predicates
(`is_ack_eliciting` / `is_probing` / `is_flow_controlled`) are
tag-only `match` dispatch with no slices and are fully covered here.

Fixed-array payload variants (PATH_CHALLENGE/RESPONSE
`[Byte; 8]`, NEW_CONNECTION_ID `[Byte; 16]`) are exercised
indirectly; direct construction is deferred pending fixed-array
literal codegen verification at the user-test boundary.

## 4. Action items landed in this branch

* `unit_test.vr` — 17 tests: is_ack_eliciting (PADDING/ACK/
  CONN_CLOSE false; PING/MAX_DATA/HANDSHAKE_DONE/STREAM true);
  is_probing (PADDING true; PING/STREAM false); is_flow_controlled
  (STREAM only); FrameError 4-variant + Eq.
* `property_test.vr` — 4 laws: STREAM flow-controlled∧ack-eliciting
  ∧¬probing; non-ack-eliciting set; ack-eliciting sample;
  only-STREAM-flow-controlled.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Per-frame decode/encode round-trip (varint) | this folder | gated on EXTSLICE-1 (catalogue §1) |
| Fixed-array variant construction (PATH_*, NEW_CONNECTION_ID) | this folder | gated on array-literal codegen |
| AckRange / EcnCounts coverage | this folder | 1h |
