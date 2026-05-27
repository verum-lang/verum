# `net/quic` audit

Module: `core/net/quic/` (~25 files) — RFC 9000 QUIC transport
protocol. The full transport stack underneath HTTP/3 (RFC 9114)
and WebTransport (W3C).

Tests cover the wire-format constants:

* QUIC version IDs — VERSION_NEGOTIATION (0x00000000),
  VERSION_1 (0x00000001 per RFC 9000 §15), VERSION_2
  (0x6B3343CF per RFC 9369).
* TransportErrorCode 17 RFC 9000 §20.1 wire constants:
  NO_ERROR through NO_VIABLE_PATH, plus CRYPTO_ERROR_BASE
  (0x0100) for TLS alerts.
* ApplicationErrorCode — opaque per-protocol numbering.
* QuicError 10-variant disjointness (ConnectionRefused /
  HandshakeFailed / TransportError / ApplicationError /
  StreamReset / IdleTimeout / StatelessReset / VersionMismatch
  / InvalidTransportParameter / Closed).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http3` + `h3` | base transport for HTTP/3. |
| `core.net.tls13` | TLS 1.3 cryptographic context. |
| `core.encoding.varint` | QUIC varint codec. |
| WebTransport (W3C) | underlying transport. |

## 2. Crate-side hardcodes

Every QUIC transport error code is IANA-registered. Drift would
break wire-level interoperability. Pinned by 17
`test_transport_error_code_*` tests + 3 version constants.

## 3. Language-implementation gaps

### §3.1 QUIC-1 — Full functional surface

Encoding, decoding, ACK handling, packet number space management,
recovery, congestion control, key updates — all gated on
end-to-end QUIC fixture at L2 specs.

### §3.2 RFC 9369 QUIC v2 support

VERSION_2 (0x6B3343CF) constant pinned but actual v2 handshake
semantics + extension differences not yet wired. Roadmap.

### §3.3 Multipath QUIC (`multipath.vr`)

IETF Multipath QUIC extension partially implemented; tests at
language level.

## 4. Action items landed in this branch

* `core-tests/net/quic/unit_test.vr` — 32 unit tests covering
  3 QUIC version IDs, 17 TransportErrorCode wire constants,
  ApplicationErrorCode construction + Eq, QuicError 4-variant
  bare + disjointness.
* `core-tests/net/quic/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| QuicFrame 24-variant payload preservation | this folder | 1 day |
| AckRange + EcnCounts record fields | this folder | 2h |
| StreamFlags 5 wire flags | this folder | 1h |
| Connection state machine variant lattice | this folder | 4h |
| Transport-parameter codec round-trip (§18) | this folder | 4h |
| Address-token sealed-AEAD round-trip | this folder | 4h |
| Stateless-reset token cross-validation | this folder | 2h |
| End-to-end QUIC handshake + 1-RTT data | language level | 2 weeks |
| RFC 9369 v2 dual-stack support | stdlib + tests | 1 week |
| Multipath QUIC scheduler | stdlib + tests | 2 weeks |
