# `net/tls13/handshake/extension` audit

Module: `core/net/tls13/handshake/extension.vr` — RFC 8446 §4.2 (and
RFC 6066/7301/6520/7250/8449/8879) extension-type codepoints.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | ClientHello/ServerHello/EncryptedExtensions extension blocks. |

## 2. Crate-side hardcodes

The 14 EXT_* codepoints (server_name=0, supported_groups=10,
signature_algorithms=13, alpn=16, record_size_limit=28,
compress_certificate=27, …) are RFC 8446 §4.2 + related-RFC IANA values,
each pinned. Drift breaks extension parsing silently.

## 3. Language-implementation findings

None. Pure `public const UInt16` codepoints — compile-time constants,
no runtime surface.

## 4. Action items landed in this branch

* `unit_test.vr` — 14 EXT_* codepoint pins + a distinctness check.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Extension encode/decode + Extension ADT round-trip | tls13/handshake | gated on codec / EXTSLICE-1 |
| supported_versions / key_share / psk extension-specific codepoints | this folder | 1h |
