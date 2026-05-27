# `net/addr` audit

Module: `core/net/addr.vr` (~1016 LOC) — IP address types
(Ipv4Addr / Ipv6Addr / IpAddr) + socket addresses
(SocketAddrV4 / SocketAddrV6 / SocketAddr) + `ToSocketAddrs`
protocol + parse + RFC-conformant predicates.

Tests cover the full algebraic surface across construction,
canonical addresses, classification predicates (RFC 1918 / 5735 /
5771 / 4291), to_u32 / from_u32 round-trip, Ipv4 + Ipv6 parsing
(happy + error paths), IpAddr discriminator, SocketAddr.parse,
and `AddrParseError` variant lattice.

Sister tests for `ToSocketAddrs` (the protocol's `to_socket_addrs`
method requires DNS resolution against a fixture) are deferred
to `vcs/specs/L2-standard/net/` where DNS-mock harness lives.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.{http,http2,http3}` | server bind addresses, client targets. |
| `core.net.dns` | A/AAAA record values. |
| `core.net.cidr` | network masks built on IP types. |
| `core.net.tcp` / `core.net.udp` / `core.net.unix` | every bind / connect uses an IP address. |
| `core.mesh.xds` | Envoy listener filter-chain addresses. |
| Application networking | every socket bind/connect call. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/...` BSD-socket FFI consumes the
4-byte big-endian wire form. Pinned by `test_to_u32_*` tests.

The `SocketAddr.V4(...)` qualified-constructor form (instead of
bare `V4(...)`) is documented as a VBC codegen workaround for
nested-record-argument miscompilation (tracked as #76 in source).
See `addr.vr:702-713`. The conformance suite calls only the
qualified form via the `SocketAddr.new_v4` / `new_v6` factory
methods, so the surface is durably tested through the canonical
client path.

## 3. Language-implementation gaps

### §3.1 `Ipv4Addr.parse` workaround for codegen bug #78

Source comment at `addr.vr:137-142` documents a codegen bug where
`&parts[i]` panics with "Slice index out of bounds". Worked around
via let-binding. Same VBC codegen family as
[[btree_pattern_match_ref_generic_class]]. Tested through the
working-on-workaround path; the underlying defect is a multi-day
VBC codegen fix.

### §3.2 `SocketAddr.parse` Char-vs-Text + Result.map_err
workarounds for codegen bug #78 / #79

Three workarounds documented in source at `addr.vr:760-808`:
1. `rsplit_once(":")` Text literal instead of `':'` char literal
   — char auto-promotion takes a different codegen branch.
2. Explicit `match` instead of `.map_err(|_| ...)?` chain —
   Result.map_err method-resolution fails when transitive
   `core.base.result` import is missing.
3. Explicit `&host` reference instead of relying on auto-borrow.

Tested through the working canonical client path. Source-side
workaround durability pinned by SocketAddr.parse error-path tests.

### §3.3 SocketAddr-variant nested-record miscompile (#76)

Documented at `addr.vr:702-713` — bare `V4(...)` instead of
`SocketAddr.V4(...)` miscompiles nested record argument as the
inner record's first FIELD value (object size 8 instead of 16).
This is the **same defect class** as
[[btree_pattern_match_ref_generic_class]] +
[[enactment_field_access_oob_2026-05-24]]: codegen loses record
layout through cross-module / variant-payload pathways and
defaults to 8-byte scalar.

The qualified form `SocketAddr.V4(...)` works because the
resolver dispatches through the constructor symbol that user
code uses. Source-side discipline is durable so long as
contributors use the `SocketAddr.new_v4` / `new_v6` factories,
which the conformance suite exclusively exercises.

### §3.4 `ToSocketAddrs` protocol — type-Iter associated bound deferred

Source comment at `addr.vr:861-869` documents that the bound
`Iterator<Item = SocketAddr>` would express "yields SocketAddrs"
properly, but the typechecker doesn't yet enforce
associated-type bindings on protocol-bounded generics (#75). The
prior form `Iterator<Item>` was a no-op (Item unbound), so the
bound is dropped entirely until #75 lands. **All three impls
already use `Iter = List<SocketAddr>`** uniformly to sidestep
the impl-method-dispatch codegen failure documented at
`addr.vr:874-881`.

Effort to add language-level fix: multi-day, gated on #75.

## 4. Action items landed in this branch

* `core-tests/net/addr/unit_test.vr` — 95 unit tests covering
  Ipv4Addr (28) + Ipv6Addr (16) + IpAddr (8) + SocketAddrV4 (4)
  + SocketAddrV6 (2) + SocketAddr (19) + AddrParseError (6) +
  parse-error paths (12) across the full public surface.
* `core-tests/net/addr/property_test.vr` — 20 property tests:
  to_u32/from_u32 round-trip identity over canonical addresses
  + 256-element low-octet sweep; predicate disjointness
  (loopback ⊥ private, broadcast ⊥ private, multicast ⊥
  broadcast); RFC 1918 boundary lattices (10/8, 172.16-31/12,
  192.168/16); multicast 224-239 boundary; Ipv6 predicate
  exclusivity (loopback ⊥ multicast, link-local ⊥ unique-local);
  SocketAddr V4 XOR V6 + port preservation sweep;
  AddrParseError 3×3 disjointness matrix.
* `core-tests/net/addr/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| `ToSocketAddrs` protocol coverage (host:port DNS path) | this folder | gated on DNS mock harness (vcs/specs/L2-standard/net/) |
| Eq/Hash/Display for IpAddr / SocketAddrV4/V6 — currently
  defined but conformance suite doesn't exercise `Map<IpAddr, _>`
  lookup | this folder | 1h once Map dispatch class closes |
| Display round-trip ∀a. parse(a.to_string()) == Ok(a) | this folder | 4h (relies on Display impl coverage in core/text/format/) |
| Sister coverage for `core.net.{cidr,ipv6_canonical,dns,link_header}` | sister folders | tracked as separate INVENTORY rows |
