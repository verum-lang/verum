# `net/quic/error` audit

Module: `core/net/quic/error.vr` — RFC 9000 §20 error surface:
`TransportErrorCode` (17 §20.1 wire constants + CRYPTO_ERROR_BASE),
`ApplicationErrorCode`, and the `QuicError` 11-variant ADT.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.frame` | CONNECTION_CLOSE carries a transport/application code. |
| `core.net.quic.connection_sm` | error → CONNECTION_CLOSE frame. |
| `core.net.tls13` | CRYPTO_ERROR = 0x0100 + TLS alert code. |

## 2. Crate-side hardcodes

All 17 TransportErrorCode constants (NO_ERROR=0x00 … NO_VIABLE_PATH=0x10)
and CRYPTO_ERROR_BASE=0x0100 are RFC 9000 §20.1 IANA wire values, each
pinned. The CRYPTO_ERROR_BASE + alert composition (§20.1) is pinned by
`test_qerr_crypto_error_base_plus_alert`.

## 3. Language-implementation findings

None. TransportErrorCode / ApplicationErrorCode are single-`UInt64`-field
records; QuicError is a mixed tuple/record/unit-variant ADT. All compile
and dispatch cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 17 TransportErrorCode constants + new/value;
  ApplicationErrorCode; QuicError variants (TransportError/StreamReset/
  IdleTimeout) + scope disjointness + Display.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| QuicError full 11-variant Eq matrix | this folder | 1h |
| CONNECTION_CLOSE frame projection round-trip | quic/frame | gated on EXTSLICE-1 |
