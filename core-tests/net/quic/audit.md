# `net/quic` audit

Module: `core/net/quic/` (~25 files) — RFC 9000 QUIC transport
protocol. The full transport stack underneath HTTP/3 (RFC 9114)
and WebTransport (W3C).

Tests cover the wire-format constants + algebraic data surface:

* QUIC version IDs — VERSION_NEGOTIATION (0x00000000),
  VERSION_1 (0x00000001 per RFC 9000 §15), VERSION_2
  (0x6B3343CF per RFC 9369).
* TransportErrorCode 17 RFC 9000 §20.1 wire constants:
  NO_ERROR through NO_VIABLE_PATH, plus CRYPTO_ERROR_BASE
  (0x0100) for TLS alerts.
* ApplicationErrorCode — opaque per-protocol numbering + Eq
  lattice (RFC 9000 §20.2).
* QuicError 10-variant disjointness (ConnectionRefused /
  HandshakeFailed / TransportError / ApplicationError /
  StreamReset / IdleTimeout / StatelessReset / VersionMismatch
  / InvalidTransportParameter / Closed) + payload-Eq for the
  three Text-carrying variants.
* QuicFrame variant disjointness sample (PingFrame /
  HandshakeDone / MaxData / RetireConnectionId / PaddingRun).
* AckRange + EcnCounts + StreamFlags field independence
  (RFC 9000 §19.3 + §19.8).
* is_supported + is_greased predicate behaviour (RFC 9000
  §15 + RFC 8701 GREASE pattern).
* FrameError 4-variant Eq + InvalidType payload preservation.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http3` + `h3` | base transport for HTTP/3. |
| `core.net.tls13` | TLS 1.3 cryptographic context (RFC 9001 §4). |
| `core.encoding.varint` | QUIC varint codec. |
| WebTransport (W3C) | underlying transport. |

## 2. Crate-side hardcodes

Every QUIC transport error code is IANA-registered. Drift would
break wire-level interoperability. Pinned by 17
`test_transport_error_code_*` tests (unit) + 5 active
`regression_quic_te_*` (regression) + 3 version constants.

## 3. Language-implementation gaps

### §3.1 QUIC-1 — Full functional surface

Encoding, decoding, ACK handling, packet number space management,
recovery, congestion control, key updates — all gated on
end-to-end QUIC fixture at L2 specs. Frame-type IDs are inlined
as literals at the codec boundary in `frame.vr` rather than
exported as `public const`; pinned via variant payload
preservation tests instead of bare-constant equality.

### §3.2 QUIC-2 — RFC 9369 QUIC v2 + Multipath support

VERSION_2 (0x6B3343CF) constant pinned but actual v2 handshake
semantics + extension differences (RFC 9369 §5 initial-salt label
change) not yet wired. Multipath QUIC (`multipath.vr`) partial
impl; tests at language level.

### §3.3 Transport-parameter codec coverage

`transport_params.vr` declares 19 parameter IDs as **module-private**
constants (`const PARAM_*: UInt64 = ...`) rather than `public const`.
This is by design (codec internals) but means parameter-ID drift is
not pinnable from this folder until either re-export or codec
round-trip lands.

## 4. Action items landed in this branch

* `core-tests/net/quic/unit_test.vr` — 32 unit tests covering
  3 QUIC version IDs, 17 TransportErrorCode wire constants,
  ApplicationErrorCode construction + Eq, QuicError 4-variant
  bare + disjointness.
* `core-tests/net/quic/property_test.vr` — 38 property tests
  spanning TransportErrorCode disjointness §A, value preservation
  §B, CRYPTO_ERROR_BASE structural §C, ApplicationErrorCode
  opacity §D, QuicError Eq + payload §E-F, version constants §G,
  is_supported / is_greased predicates §H, AckRange / EcnCounts
  / StreamFlags field independence §I-J, QuicFrame sample §K,
  FrameError §L.
* `core-tests/net/quic/regression_test.vr` — 15 regression pins
  (9 active wire-format LOCK-IN over version + transport error
  codes + predicate behaviour, 6 `@ignore`'d functional pins
  for frame codec + transport params + v2 handshake + multipath).
* `core-tests/net/quic/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| QuicFrame 24-variant payload preservation full coverage | this folder | 1 day |
| StreamFlags 5 wire-bit dispatch | this folder | 1h |
| Connection state machine variant lattice | this folder | 4h |
| Transport-parameter codec round-trip (RFC 9000 §18) | this folder | 4h, gated on §3.3 |
| Address-token sealed-AEAD round-trip | this folder | 4h |
| Stateless-reset token cross-validation | this folder | 2h |
| End-to-end QUIC handshake + 1-RTT data | language level | 2 weeks |
| RFC 9369 v2 dual-stack support | stdlib + tests | 1 week |
| Multipath QUIC scheduler | stdlib + tests | 2 weeks |
