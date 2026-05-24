# `security/jwt` audit

Module: `core/security/jwt.vr` (~700 LOC) — JSON Web Token
implementation per RFC 7519 + RFC 7515. Defines JwtAlgorithm
4-variant + JwtKey 3-variant + JwtError 12-variant + JwtHeader +
JwtPayload + sign/verify functions.

Tests cover the static surface: JwtAlgorithm + JwtError variant
construction + disjointness. Live sign/verify requires HMAC +
Ed25519 backend operations, tested at language level.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| Application auth code | `jwt.sign(payload, key, algorithm)` + `jwt.verify(token, allowed_algs, key)`. |
| `core.net.http` middleware | bearer-token validation. |
| OAuth flows | bearer token issuance. |

## 2. Crate-side hardcodes

None today — pure Verum JWT implementation. HMAC + Ed25519
intrinsics consumed via core.security.mac.hmac + ecc.ed25519.

## 3. Language-implementation gaps

### §3.1 Security-critical contracts pinned in tests

* `JwtError.AlgNone` — CVE-2015-2951 explicit rejection of
  "alg":"none" tokens, even when caller asks. The variant exists
  separately from SignatureMismatch so callers can distinguish
  the attack vector in their security audit logs.
* `JwtError.WrongKeyKind` — algorithm-confusion prevention: HMAC
  key cannot be passed to asymmetric verifier (and vice versa).
  The variant is separate from SignatureMismatch.
* These two security-critical variants are pinned distinct in
  this branch's tests so a future refactor that collapses them
  surfaces immediately.

### §3.2 Add Display/Debug/Eq for JwtAlgorithm + JwtError

Surface already qualifies match arms (good). Add the protocol
impls per the established stdlib discipline.

**Effort:** small (~30 min) — 4 + 12 variants.

### §3.3 Add ES256 (ECDSA-P256) once stdlib P-256 lands

Documented as follow-up in jwt.vr:31-32. JwtAlgorithm enum needs
ES256 variant + signing path. Tracked separately.

### §3.4 Live sign/verify tests at L2

Multi-step crypto operations need real HMAC + Ed25519. Cross-
reference `vcs/specs/L2-standard/security/jwt/`.

## Action items landed in this branch

* `core-tests/security/jwt/unit_test.vr` — 18 unit tests
  covering JwtAlgorithm 4-variant + JwtError 12-variant +
  security-critical variant distinctness pins (AlgNone vs
  SignatureMismatch vs WrongKeyKind, Expired vs NotYetValid).
* `core-tests/security/jwt/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Display/Debug/Eq for JwtAlgorithm + JwtError | `core/security/jwt.vr` + tests | 30 min |
| Add ES256 (ECDSA-P256) once stdlib P-256 lands | `core/security/jwt.vr` | gated |
| Live sign/verify tests | `vcs/specs/L2-standard/security/jwt/` | 1 day |
| Sister tests for `core.security.{aead,cipher,encrypt,hash,kdf,ecc,hpke,cose,webauthn}` | sister folders | 1 week total |
EOF
