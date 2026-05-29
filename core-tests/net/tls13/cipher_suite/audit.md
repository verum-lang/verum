# `net/tls13/cipher_suite` audit

Module: `core/net/tls13/cipher_suite.vr` — RFC 8446 §B.4 cipher
suites: `CipherSuite` 5-variant + to_u16/from_u16 (IANA codepoints)
+ hash_kind / aead_kind accessors.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | ClientHello/ServerHello cipher_suites list. |
| `core.net.tls13.keyschedule` | hash_kind selects the HKDF hash. |
| `core.net.tls13.record` | aead_kind selects the record AEAD. |

## 2. Crate-side hardcodes

The 5 IANA codepoints (TLS_AES_128_GCM_SHA256=0x1301 …
TLS_AES_128_CCM_8_SHA256=0x1305) are RFC 8446 §B.4 verbatim. The
hash binding (AES-256-GCM → SHA-384; all others → SHA-256) is pinned.

## 3. Language-implementation findings

None. 5-variant unit enum + to_u16/from_u16 + accessor `match`.
Compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 5 codepoint pins + from_u16 + hash_kind (3 cases).
* `property_test.vr` — from_u16 ∘ to_u16 == id over all 5 suites.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| aead_kind per-suite coverage | this folder | 30min |
| is_supported policy pins | this folder | 30min |
