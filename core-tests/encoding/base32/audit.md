# `encoding/base32` audit

Module: `core/encoding/base32.vr` (503 LOC) — RFC 4648 §6 Base32
codec (A-Z + 2-7) + RFC 4648 §7 base32-hex extension.

Tests: `unit_test.vr` (~22 unit tests covering encode RFC §10
canonical fixtures + round-trip + case-insensitive decode +
decode_no_pad for OTP + 2 error paths + Base32Error 3-variant).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.security.totp` | shares secrets via base32 (Google Authenticator). |
| `core.security.hotp` | RFC 4226 HOTP secret encoding. |
| `core.net.dns` | DNSSEC NSEC3 hashed-owner-name encoding. |
| `core.net.onion` | Tor v3 onion-service 32-byte addresses → 56-char domain. |

## 2. Crate-side hardcodes

None today — pure Verum codec.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified Display/Debug/Eq arms

Source-side fix in this round.

### §3.2 No property_test.vr round-trip law sweep

Add: ∀b. decode(encode(b)) == Ok(b) over representative samples
(empty, 1, 3, 5, 7-byte boundaries — encoding output length
multiple-of-8 invariants).

**Effort:** small (~30 min).

### §3.3 base32-hex variants need coverage

`encode_hex` / `decode_hex` / `decode_hex_no_pad` are tested as
existing in surface but not exercised in this unit_test.

**Effort:** small (~30 min).

## Action items landed in this branch

* `core/encoding/base32.vr` — qualified Display/Debug/Eq arms.
* `core-tests/encoding/base32/unit_test.vr` — 22 unit tests.
* `core-tests/encoding/base32/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (round-trip + output-length law) | this folder | 30 min |
| Add base32-hex (encode_hex / decode_hex) unit tests | this folder | 30 min |
| Sister tests for `core.encoding.base58` Bitcoin alphabet | core-tests/encoding/base58/ | 1-2h |
EOF
