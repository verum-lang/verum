# `net/tls` audit

Module: `core/net/tls.vr` (~1114 LOC) — TLS 1.0-1.3 streams +
connector + acceptor + certificate / private-key types + system
certificate store. Pluggable backend model
(rustls-ktls / openssl-fips / hacl-star / SChannel /
SecureTransport).

Tests cover the algebraic data-surface:

* `TlsVersion` 4-variant + as_str (TLS 1.0/1.1/1.2/1.3) +
  wire_version (3,1/3,2/3,3/3,4) + is_secure (TLS 1.2/1.3 only).
* `KeyType` 3-variant + as_str (RSA / EC / Ed25519).
* `CertVerifyMode` 3-variant (Full / None / Custom).
* `Certificate.from_der` record construction.
* `PrivateKey.from_der` + key_type accessor.

Full TLS handshake / read / write requires the pluggable
backend at run-time — tested at language level
(`vcs/specs/L2-standard/net/tls/`).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http` HTTPS clients | TlsStream.connect over TcpStream. |
| `core.net.weft` server | TlsStream.accept on listener. |
| `core.net.websocket` | wss:// over TlsStream. |
| `core.security.auth-primitives` channel binding | `peer_cert_hash_sha256` for SCRAM-SHA-256-PLUS (RFC 5929 tls-server-end-point). |
| `core.net.tls13` | sibling module — pure-Verum TLS 1.3 reference impl planned to replace this. |

## 2. Crate-side hardcodes

`@intrinsic("verum.tls.*")` symbols dispatch to backend at link
time (rustls-ktls / openssl-fips / hacl-star / SChannel /
SecureTransport). The wire-version (major, minor) tuples are
RFC-stable and pinned by `test_tls_version_wire_*` (RFC 8446
§4.1.2 / RFC 5246 §6.2.1 for legacy versions).

## 3. Language-implementation gaps

### §3.1 TLS-1 — Backend pluggability gated on link-time symbol resolution

Source-side at `tls.vr:896-907` lists backend matrix; all
backends marked **planned**. Live TLS tests require a backend
implementation; data-surface tests pass.

### §3.2 TlsVersion.Tls13.wire_version returns (3, 4)

RFC 8446 §5.1: TLS 1.3 ClientHello/ServerHello use the legacy
record version `(3, 3)` on the wire + signal `0x0304` via
supported_versions extension. The `wire_version` method
returns `(3, 4)` — this matches the version record encoding
but not the legacy ClientHello field. Source-side comment
documents this discrepancy.

**Note**: `test_tls_version_wire_tls13` pins (3, 4) as the
canonical version encoding; the legacy ClientHello-record
field shape is a backend implementation detail.

### §3.3 `CertVerifyMode.None` — DANGEROUS marker

Source-side at `tls.vr:305` explicitly marks `None` as
"DANGEROUS - for testing only". The conformance suite pins
construction of the variant without exercising live no-verify
handshakes (which require fixture certificates + TOCTOU-safe
fixture lifecycle).

### §3.4 Post-quantum hybrid X25519MLKEM768 + RFC 8879 cert compression — roadmap items

Source-side at `tls.vr:909-916` lists 6 roadmap items: PQC
hybrid KEM, ECH, 0-RTT, cert compression, session tickets,
SNI multi-tenant. None implemented; deferred.

## 4. Action items landed in this branch

* `core-tests/net/tls/unit_test.vr` — 31 unit tests covering
  TlsVersion (4 ctor + 4 as_str + 4 wire_version + 4 is_secure
  + 3 Eq/disjoint), KeyType (3 ctor + 3 as_str + 2 disjoint),
  CertVerifyMode (3 ctor + 1 disjoint), Certificate.from_der
  (2: empty + dummy DER bytes), PrivateKey.from_der + key_type
  (3 ctor + accessor for RSA/EC/Ed25519).
* `core-tests/net/tls/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TlsConfig.client/.server fluent builder coverage | this folder | 4h |
| Certificate.from_pem / PrivateKey.from_pem round-trip with sample fixtures | this folder + fixtures | 4h once fixture model lands |
| LegacyTlsError 9-variant Eq + disjointness | this folder | 1h |
| Full TLS handshake round-trip against in-process backend fixture | language level | 1 week (backend dependent) |
| Post-quantum hybrid X25519MLKEM768 default per IETF mandate | stdlib + tests | 2 weeks (depends on backend) |
| ECH (Encrypted ClientHello) HPKE over HTTPS DNS records | stdlib + tests | 2 weeks |
| Session ticket resumption (RFC 8446 §4.6.1) | stdlib + tests | 1 week |
