# X.509 Differential Harness — scaffolding

Cross-implementation X.509 DER parser parity harness per
`internal/specs/tls-quic.md` §2.5.

## Planned scope

| Layer | Reference 1 | Reference 2 | Entry point |
|-------|-------------|-------------|-------------|
| DER parse | rustls-webpki | OpenSSL 3.x | `golden/` |
| Chain verify (RFC 5280 §6) | rustls-webpki | OpenSSL | `chain/` |
| OCSP response parse | rustls-webpki | OpenSSL | `ocsp/` |

## Invariants

* `der_decode(encode(x)) == x` — byte-exact round-trip.
* Chain verification produces the same verdict (pass/fail + error
  code class) on 10K random chains harvested from WebPKI-connected
  hosts (Let's Encrypt / DigiCert / Amazon / Google CA chains).
* OCSP response validation matches rustls-webpki on the IETF
  `Stapling Corpus` (~500 captures).

## Bootstrap status

`core/security/x509/` ships DerReader + Certificate + chain + name +
spki + algorithm modules; V10 theorem (nonempty + pairwise-edge +
self-signed anchor + validity window) is SMT-discharged under
`vcs/specs/L2-standard/security/x509/v10_chain_validation_theorem.vr`.

This harness becomes live once `vcs/differential/x509/runners/{rustls-webpki,openssl}.sh`
are provided (docker-based, outside this directory).
