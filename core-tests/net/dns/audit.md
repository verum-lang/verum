# `net/dns` audit

Module: `core/net/dns.vr` (~1928 LOC) — pure-Verum DNS resolver
per RFC 1035 over UDP with TCP fallback. The largest single
file in `core/net/`.

Tests cover the algebraic data-surface: 10 DNS wire-type
constants (RFC 1035 §3.2.2) + DnsRecordType 10-variant +
DnsError 13-variant disjointness + DnsRecord 9-variant payload
preservation + DnsRecordEntry record (record + ttl pair).

Live network paths (`lookup_host`, `lookup_host_v4`,
`lookup_host_v6`, `lookup_addr`, `resolve`, `Resolver.query` +
async variants) are tested at language level
(`vcs/specs/L2-standard/net/dns/`) against mock resolver fixtures
— pinning these here would require either live internet access
(non-hermetic CI gate) or a stdlib-level mock harness (not yet
present).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.addr.ToSocketAddrs` for `(&Text, Int)` and `Text` impls | DNS resolution of hostname → SocketAddr |
| `core.net.{tcp,udp,unix}` connect | hostname → IP resolution via `resolve` |
| `core.net.url` host field | NOT used (URL parser doesn't resolve) |
| Application networking | every connect() / bind() against a hostname |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/dns/...` UDP socket FFI uses the
10 DNS_TYPE_* constants as direct wire-format values. Drift here
would produce broken DNS queries. Pinned by `test_dns_type_*`
tests (10 pins covering A/NS/CNAME/SOA/PTR/MX/TXT/AAAA/SRV/ANY).

## 3. Language-implementation gaps

### §3.1 DNS-1 — live `lookup_*` / `resolve` paths require mock harness

Stable trigger: any call to `lookup_host(&Text)` /
`lookup_host_v4` / `lookup_host_v6` / `lookup_addr` / `resolve`
or `Resolver.query` against a real hostname requires UDP/TCP
network access. The data-surface compiles; the live functional
surface is gated on:

1. **DNS-mock test harness** — would land in
   `vcs/specs/L2-standard/net/dns/` with a pre-canned UDP-
   wire-fixture response.
2. **Live-mode test marker** — `@slow`/`@network` annotation
   to gate against non-hermetic CI.

Pre-fix infrastructure does not exist in `core-tests/`. The
data-surface coverage in this folder is the conformance-suite
plumb-line; functional coverage lives at L2 specs.

### §3.2 `DnsError.from_io_error` IoErrorKind ↔ DnsError mapping

Source-side at `dns.vr:218-227`. Mapping documented:

| IoErrorKind | DnsError |
|---|---|
| `TimedOut` | `Timeout` |
| `NotFound` | `HostNotFound` |
| `ConnectionRefused` | `Refused` |
| `WouldBlock` / `Interrupted` | `TryAgain` |
| (any other) | `NetworkError(formatted)` |

Cross-validation tests need IoError fixture which requires
runtime instrumentation — deferred.

### §3.3 `is_valid_domain` / `is_ip_address` byte-helpers

Source-side at `dns.vr:1811` + `dns.vr:1823`. Public free fns
that are likely SIGSEGV under user-side compilation per the
CIDR-1 family precompile-cascade defect class. Locked-in only
indirectly — the data-surface algebra doesn't reach them.

## 4. Action items landed in this branch

* `core-tests/net/dns/unit_test.vr` — 37 unit tests covering
  10 DNS_TYPE_* wire-format constants (canonical RFC 1035
  values pinned individually), 10 DnsRecordType variants +
  A-vs-AAAA disjointness, DnsError 13-variant Eq + 3
  pairwise-disjointness, 9 DnsRecord variants (A/AAAA/CNAME/
  MX/TXT/NS/PTR/SRV/SOA payload preservation), 3
  DnsRecordEntry record + TTL boundary (zero / max-Int).
* `core-tests/net/dns/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| DNS mock-resolver harness at L2 specs | vcs/specs/L2-standard/net/dns/ | 1 day |
| Live `lookup_host` happy + error path tests | this folder + @network marker | 4h, gated on harness |
| from_io_error IoErrorKind → DnsError mapping coverage | this folder | 1h, gated on IoError fixture |
| Resolver builder (nameserver_ip / timeout_ms / max_retries / cache_clear) | this folder | 2h |
| RFC 1035 §4.1.4 label-compression encoding/decoding | language level | 1 day |
| RFC 6891 EDNS0 OPT record support | stdlib + tests | 1 day |
