# `net/addr` audit

Module: `core/net/addr.vr` (~1000 LOC) — IP address types
(Ipv4Addr / Ipv6Addr) + SocketAddr / SocketAddrV4 / SocketAddrV6 +
parsing + RFC-conformant predicates.

Tests cover Ipv4Addr static surface: constructors, canonical
addresses, classification predicates (is_loopback / is_unspecified /
is_private / is_multicast / is_broadcast per RFC 1918 / 5735 /
5771), to_u32 / from_u32 round-trip.

Ipv6Addr + SocketAddr* tests deferred — larger surface, follow-up
session.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.{http,http2,http3}` | server bind addresses, client targets. |
| `core.net.dns` | A/AAAA record values. |
| `core.net.cidr` | network masks built on IP types. |
| `core.mesh.xds` | Envoy listener filter-chain addresses. |
| Application networking | every socket bind/connect call. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/...` BSD-socket FFI consumes the
4-byte big-endian wire form. Pinned by `test_to_u32_*` tests.

## 3. Language-implementation gaps

### §3.1 `Ipv4Addr.parse` workaround for codegen bug #78

Source comment at `addr.vr:137-142` documents a codegen bug where
`&parts[i]` panics with "Slice index out of bounds". Worked around
via let-binding. This is task #17/#39 territory or its sibling
slice-deref hazard. Test the workaround is durable.

### §3.2 `Ipv4Addr.parse` doesn't unit-test in this folder

The parse error paths + happy paths are not exercised in this
suite — focus is on the algebraic surface. Add follow-up
property_test.vr for ∀a. parse(a.display()) == Ok(a).

**Effort:** small (~1h).

### §3.3 No Eq / Display / Hash for Ipv4Addr

`Map<Ipv4Addr, BanState>` lookups need Hash. Display for
`f"{addr}"` rendering. Add following the Method/Color pattern.

**Effort:** small (~30 min).

### §3.4 Sister coverage deferred — Ipv6Addr + SocketAddr*

Ipv6Addr is a 16-byte segments record; SocketAddrV4/V6 are
(addr, port) tuples. Full RFC 5952 canonical Ipv6 form is
covered separately in `core/net/ipv6_canonical.vr`.

## Action items landed in this branch

* `core-tests/net/addr/unit_test.vr` — 28 unit tests covering
  Ipv4Addr construction, canonical addresses (localhost/
  unspecified/broadcast), classification predicates per RFC
  1918/5735/5771, u32 round-trip.
* `core-tests/net/addr/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Ipv4Addr.parse unit tests (happy + error paths) | this folder | 1h |
| Add Eq / Display / Hash for Ipv4Addr | `core/net/addr.vr` + tests | 30 min |
| Add Ipv6Addr + SocketAddrV4/V6 + SocketAddr coverage | this folder | 1 day |
| Add property_test.vr (parse∘display round-trip, RFC predicate disjointness) | this folder | 1h |
| Sister tests for `core.net.{cidr,ipv6_canonical,dns,link_header}` | sister folders | 1 day total |
EOF
