# `net/quic/packet` audit

Module: `core/net/quic/packet.vr` — RFC 9000 §17 packet framing:
`LongPacketType` 4-variant + to_bits (§17.2 long-header type bits),
`LongHeader`/`ShortHeader`/`Packet` structures, `PacketError`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | packet demux on the receive path. |
| `core.net.quic.crypto` | header protection + packet-number encoding. |

## 2. Crate-side hardcodes

LongPacketType bits (Initial=0b00, ZeroRtt=0b01, Handshake=0b10, Retry=0b11)
are RFC 9000 §17.2 verbatim and pinned + verified pairwise-distinct + 2-bit
bounded.

## 3. Language-implementation findings

None for the covered surface. LongPacketType is a 4-variant unit enum +
to_bits `match` — compiles cleanly under `--interp`. Packet encode/decode
(header protection + varint + sub-slice) is gated on EXTSLICE-1 + crypto.

## 4. Action items landed in this branch

* `unit_test.vr` — 4 type-bit pins + pairwise distinctness + 2-bit bound.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Long/short header encode/decode round-trip | this folder | gated on EXTSLICE-1 + crypto |
| PacketError variant Eq matrix | this folder | 1h |
| Packet-number encode/decode (1-4 byte) | this folder | gated on codec |
