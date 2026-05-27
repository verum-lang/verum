# `net/tls13` audit

Module: `core/net/tls13/` (~2500 LOC across 12 files) — pure-
Verum TLS 1.3 (RFC 8446) implementation. Phase 2 of the
`net.tls` replacement plan — will subsume `core.net.tls` once
session.* coverage stabilises.

Tests cover the algebraic data-surface for the IANA-registered
wire identifiers and parameter tables:

* `ProtocolVersion` record — `TLS_1_3 = (3, 4)`,
  `LEGACY_VERSION_TLS_1_2 = (3, 3)`.
* `CipherSuite` 6-variant — RFC 8446 §B.4 IDs `0x1301`-`0x1305`
  + `Unknown(UInt16)` fallthrough, `is_supported` per §9.1
  mandatory-to-implement list, `hash_kind` → Sha256 / Sha384
  routing, `aead_kind` AEAD selector.
* `HashKind` 2-variant + `output_len` (Sha256=32 / Sha384=48).
* `AeadKind` 6-variant + `key_len` + `iv_len` (always 12 for
  TLS 1.3) + `tag_len` (16 default + 8 for CCM-8).
* `NamedGroup` 9-variant — X25519 + Secp256r1/384r1/521r1 +
  X448 + FFDHE2048/3072/4096 + Unknown, `is_supported` v1
  production list, `public_key_len` (X25519=32 / X448=56).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http3` + `quic` | TLS 1.3 is the only TLS version permitted in QUIC (RFC 9001 §4). |
| `core.security.{x509, hash, mac, kdf, aead, ecc, sig}` | crypto primitives. |
| Application TLS-aware clients | session ticket caching + 0-RTT. |

## 2. Crate-side hardcodes

None — `core/net/tls13/*` is pure Verum. The crypto primitives
in `core.security.*` are intrinsic-backed; this module is the
TLS state machine + record layer + key schedule.

## 3. Language-implementation gaps

### §3.1 TLS13-1 — Session / Handshake state machines deferred

Source-side at `mod.vr:40-46` documents Phase 2 step 1 (key
schedule + record layer + wire constants — what this test suite
covers) vs Phase 2 step 2 (handshake state machine — landing in
follow-up). The `session.*` re-exports are present in the
umbrella but tested at L2 specs end-to-end.

### §3.2 RFC 8879 Certificate Compression — file shipped without umbrella

Source-side at `mod.vr:88-92` documents that
`cert_compress.vr` shipped pre-fix without an umbrella module
declaration; reachable via direct file mount only. The
umbrella was added at `mod.vr:92` but external visibility was
gated until that line landed.

### §3.3 v1 supported `NamedGroup` list excludes Secp521r1 + X448 + FFDHE

Source-side at `named_group.vr:74-81` — only X25519 / P-256 /
P-384 are negotiated. The other variants parse correctly but
return false from `is_supported`. Tested via
`test_named_group_x448_is_not_supported_v1`.

### §3.4 v1 supported CipherSuite excludes both CCM variants

Source-side at `cipher_suite.vr:72-79` — only Aes128Gcm /
Aes256Gcm / ChaCha20Poly1305 negotiate. Aes128Ccm + Aes128Ccm8
parse correctly but return false from `is_supported`. Tested
via `test_cipher_suite_ccm_is_not_supported`.

## 4. Action items landed in this branch

* `core-tests/net/tls13/unit_test.vr` — 39 unit tests covering
  ProtocolVersion record fields (5), CipherSuite 6-variant
  IANA u16 wire IDs (5) + to_u16/from_u16 round-trip (3) +
  is_supported (4) + hash_kind (3), HashKind output_len (2),
  AeadKind key_len/iv_len/tag_len (5), NamedGroup 6-variant
  IANA u16 wire IDs (6) + is_supported (4) + public_key_len
  (2).
* `core-tests/net/tls13/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| SignatureScheme catalogue (RSA-PSS / ECDSA / EdDSA) data-surface | this folder | 1h |
| AlertLevel + AlertDescription wire-form (RFC 8446 §6) | this folder | 1h |
| TlsError ADT + alert mapping | this folder | 1h |
| ContentType wire constants (RFC 8446 §B.1) | this folder | 30 min |
| AeadState seal/open round-trip with fixture key | this folder | 4h |
| HKDF-Expand-Label test vectors per RFC 8446 §7.1 | this folder | 4h |
| Handshake state-machine ClientHello → Finished happy-path | language level | 1 week |
| 0-RTT session ticket round-trip | language level | 1 week |
| RFC 8879 cert compression Brotli + Zstd round-trip | this folder | 4h |
