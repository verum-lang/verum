# `net/websocket` audit

Module: `core/net/websocket.vr` (~510 LOC) — RFC 6455 WebSocket
protocol: opening-handshake helpers, frame codec with masking,
close codes, control frames (Ping / Pong / Close), fragmentation.
Permessage-deflate (RFC 7692) tracked as follow-up.

Tests cover the algebraic data-surface:

* `WsOpcode` 7-variant + to_byte / from_byte round-trip +
  is_control + is_data predicates (RFC 6455 §5.2).
* `CloseCode` RFC 6455 §7.4 well-known constants (1000 NORMAL,
  1001 GOING_AWAY, 1002 PROTOCOL_ERROR, 1003 UNSUPPORTED_DATA,
  1007 INVALID_FRAME_PAYLOAD, 1008 POLICY_VIOLATION, 1009
  MESSAGE_TOO_BIG, 1011 INTERNAL_ERROR) + new + code accessor.
* `WsFrame` factory ctors (text / binary) with FIN/RSV/opcode
  field preservation.

`encode(&WsFrame, mask_key, &mut out)`, `decode(buf, expect_masked,
max_payload) -> WebSocketDecodeResult`, `accept_key`,
`validate_server_handshake` functional paths are subject to the
precompile-cascade SIGSEGV class shared with CIDR-1 family — not
explicitly pinned here because data-surface coverage is the
primary correctness signal.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` SSE/WebSocket adapter | accept-loop integration. |
| `core.net.http` Upgrade handshake | HTTP/1.1 → WebSocket transition. |
| Application bidirectional streaming clients | every chat/notification feed. |

## 2. Crate-side hardcodes

`WsOpcode.to_byte()` returns RFC 6455 §5.2 wire constants
directly — drift would produce non-RFC frames. Pinned by 7
`test_ws_opcode_to_byte_*` tests.

`CloseCode` constants (1000-1011) are pinned by 8
`test_close_code_*` tests — these are IANA-registered values per
RFC 6455 §7.4.

## 3. Language-implementation gaps

### §3.1 WS-1 — `encode` / `decode` / `accept_key` /
       `validate_server_handshake` functional paths

Subject to the precompile-cascade SIGSEGV class (CIDR-1 family).
Data-surface tests pass; live frame codec round-trips are at
L2 specs.

### §3.2 RFC 7692 permessage-deflate — not implemented

Source-side at `websocket.vr` header documents permessage-deflate
as follow-up. RSV1 bit currently fails with `ReservedBitsSet`
during decode.

### §3.3 `WEBSOCKET_MAGIC` 36-byte GUID public constant

Used in `accept_key` to derive Sec-WebSocket-Accept per RFC 6455
§4.2.2 (SHA-1 of `Sec-WebSocket-Key` + magic GUID). Public
constant for caller-side handshake.

### §3.4 Control-frame size limit

RFC 6455 §5.5 mandates control-frame payload ≤ 125 bytes +
no fragmentation. The decoder enforces this — tested at L2 specs.

## 4. Action items landed in this branch

* `core-tests/net/websocket/unit_test.vr` — 38 unit tests
  covering WsOpcode 7-variant construction + to_byte (7) +
  from_byte (4) + is_control/is_data (7) + CloseCode 8 RFC
  6455 constants + new/code (3) + WsFrame text/binary factory
  (2).
* `core-tests/net/websocket/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| WsFrame.ping/pong/close factory ctors + payload-size invariant | this folder | 1h |
| WebSocketDecodeError variant Eq + disjointness | this folder | 1h |
| WebSocketDecodeResult 3-variant (NeedMore / Decoded / Err) | this folder | 30 min |
| encode / decode round-trip for text + binary frames | this folder | 2h, gated on WS-1 |
| accept_key against RFC 6455 §1.3 test vector "dGhlIHNhbXBsZSBub25jZQ==" → "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" | this folder | 1h, gated on WS-1 |
| validate_server_handshake against canonical Upgrade headers | this folder | 1h, gated on WS-1 |
| RFC 7692 permessage-deflate | stdlib + tests | 1 week |
| Control-frame size + fragmentation invariants property tests | this folder | 2h |
