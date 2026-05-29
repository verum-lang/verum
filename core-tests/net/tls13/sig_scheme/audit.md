# `net/tls13/sig_scheme` audit

Module: `core/net/tls13/sig_scheme.vr` — RFC 8446 §4.2.3
signature_algorithms: `SignatureScheme` + to_u16/from_u16 (IANA
codepoints) + `Unknown(UInt16)` passthrough.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | signature_algorithms extension + CertificateVerify. |

## 2. Crate-side hardcodes

The IANA codepoints (ecdsa_secp256r1_sha256=0x0403, rsa_pss_rsae_*=0x0804-6,
ed25519=0x0807, ed448=0x0808, rsa_pss_pss_*=0x0809-B, legacy rsa_pkcs1_*,
ecdsa_sha1=0x0203) are RFC 8446 §4.2.3 verbatim. `Unknown(v)` round-trips
any unrecognised codepoint, so the round-trip law holds over the whole
16-bit space.

## 3. Language-implementation findings

None. Mixed unit + `Unknown(UInt16)` enum + to_u16/from_u16 `match`.
Compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 9 codepoint pins + from_u16 + Unknown preservation.
* `property_test.vr` — round-trip over the 16 IANA codepoints + Unknown
  round-trip over arbitrary values.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TLS-1.3-permitted subset classification | this folder | 30min |
