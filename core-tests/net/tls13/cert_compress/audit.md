# `net/tls13/cert_compress` audit

Module: `core/net/tls13/cert_compress.vr` — RFC 8879 TLS certificate
compression: `CertCompressionAlgorithm` + to_u16/from_u16 + is_registered,
`HST_COMPRESSED_CERTIFICATE` handshake type.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | compress_certificate extension + CompressedCertificate msg. |

## 2. Crate-side hardcodes

Algorithm codepoints Zlib=1, Brotli=2, Zstd=3 (RFC 8879 §7.3) and the
CompressedCertificate handshake type = 25 are pinned. `UnknownAlgo(v)`
round-trips arbitrary codepoints; is_registered distinguishes the 3 IANA
algorithms from unknowns.

## 3. Language-implementation findings

None. Mixed unit + `UnknownAlgo(UInt16)` enum + to_u16/from_u16 +
is_registered `match`. Compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 3 codepoint pins + HST type; from_u16 + Unknown
  passthrough + is_registered (registered/unknown) + round-trip.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Actual zlib/brotli/zstd compression round-trip | this folder | gated on codec |
