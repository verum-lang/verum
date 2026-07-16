# `net/ipv6_canonical` audit

Module: `core/net/ipv6_canonical.vr` (~239 LOC) — RFC 5952
canonical IPv6 text representation. Three public entry points:

* `format_ipv6(&Ipv6Addr) -> Text` — emit canonical form.
* `canonicalize(&Text) -> Result<Text, Ipv6CanonicalError>` —
  parse + re-emit.
* `equal_addresses(&Text, &Text) -> Bool` — equivalence modulo
  spelling.

Tests cover RFC 5952 §4.1 (leading-zero suppression), §4.2.1
(longest zero-run compression), §4.2.2 (single-zero NOT
compressed), §4.2.3 (leftmost on ties), §4.3 (lowercase hex),
and the round-trip + equality surfaces (`canonicalize` over
full / compressed / uppercase / unspecified + `equal_addresses`
across spelling variants and invalid-input fallback to false).

§5 IPv4-mapped form is locked-in behind IPV6CAN-1 in
`regression_test.vr` — see §3.1 below.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` access-log dedup | `equal_addresses` for log-key normalization. |
| `core.net.dns` reverse-resolve (`lookup_addr`) | requires canonical form for reverse-zone lookup. |
| Application allowlist comparison | `canonicalize` for stable hash key. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic; `Text.from_utf8_unchecked`
boundary call is the only entry into Text construction.

## 3. Language-implementation gaps

> **Defect-class catalogue**: IPV6CAN-1 is the
> [BSTRLIT-1](website:docs/stdlib/defect-class-catalogue.md#2-bliteral-byte-string-literal-sigsegv)
> byte-string-literal class.

### §3.1 IPV6CAN-1 — `format_v4_mapped` precompile SIGSEGV (CLOSED 2026-05-28)

**Pre-fix stable trigger**: calling `format_ipv6(&a)` where `a`
is v4-mapped (any address whose first 5 segments are 0 and
segment[5] == 0xFFFF) SIGSEGV'd inside
`llvm::SmallVectorBase<unsigned long long>::grow_pod`.

**Root cause confirmed**: candidate #1 — byte-string literal
`b"::ffff:"` in `push_bytes(&mut out, b"::ffff:")` triggered
the VBC codegen SIGSEGV. The other two candidates
(high-index octet access + push_decimal_byte arithmetic) were
NOT the trigger.

**Source-side fix landed 2026-05-27** (commit `8233fad28`):
inline the 7-byte prefix as individual `out.push()` calls:

```verum
out.push(':' as Byte); out.push(':' as Byte);
out.push('f' as Byte); out.push('f' as Byte);
out.push('f' as Byte); out.push('f' as Byte);
out.push(':' as Byte);
```

**Post-rebuild validation 2026-05-28**: 3/3 regression tests
transition from @ignore'd-SIGSEGV to GREEN under `--interp`.
@ignore markers removed in regression_test.vr; defect class
closed.

**Fix path**: 1-day diagnosis to isolate which of the three
candidates is the trigger, then VBC codegen edit + rebuild.
Source-side workaround would be to inline the v4-mapped logic
without the byte-string literal and pass the high-index octets
as separate Byte parameters — would defer the underlying defect
class.


### §3.2 `canonicalize` cascades to `Ipv6Addr.parse` workarounds

`canonicalize` calls `parse` which delegates to `Ipv6Addr.parse`
— the latter has documented VBC codegen workarounds at
`addr.vr:760-808` (see `net/addr/audit.md §3.2`). The
conformance suite exercises the working canonical path.

## 4. Action items landed — net-conformance-20260705

* `core-tests/net/ipv6_canonical/property_test.vr` — 15 algebraic
  laws over a structural corpus (leading/interior/trailing/whole
  zero-runs, single-zero non-compression §4.2.2, tie→leftmost
  §4.2.3, maximal groups, v4-mapped): idempotence, fixpoint,
  spelling-class collapse, `canonicalize = format_ipv6 ∘ parse`
  coherence, `equal_addresses` reflexive/symmetric/discriminating/
  invalid-false, lowercase invariant (§4.3), and both v4-mapped
  branches (dotted-tail preservation + hex-spelling → dotted).
* `core-tests/net/ipv6_canonical/integration_test.vr` — 8 cross-type
  scenarios against `core.net.addr.Ipv6Addr`: constructor → canonical
  text, Display (uncompressed) vs `format_ipv6` (compressed)
  coherence via `equal_addresses`, `Ipv6Addr.parse ∘ canonicalize`
  round-trip, and predicate survival (`is_link_local` / `is_loopback`)
  through canonicalisation.

## Legacy action items — original landing branch

* `core-tests/net/ipv6_canonical/unit_test.vr` — 23 unit tests
  covering format_ipv6 (RFC 5952 §4.1/§4.2.1/§4.2.2/§4.2.3/§4.3
  exhaustively) + canonicalize (parse + re-emit, full /
  compressed / uppercase / unspecified / invalid) +
  equal_addresses (compressed-vs-full + case-insensitive +
  disjoint + invalid fallback) — 100% of the non-v4-mapped
  semantic surface.
* `core-tests/net/ipv6_canonical/regression_test.vr` — 3
  @ignore'd LOCK-IN pins for IPV6CAN-1: `::ffff:0.0.0.0`,
  `::ffff:192.168.1.1`, `::ffff:255.255.255.255` to lock-in
  the v4-mapped defect shape.
* `core-tests/net/ipv6_canonical/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close IPV6CAN-1 (v4-mapped emit-path codegen) | VBC codegen | 3-5 days incl rebuild |
| Display/Debug for Ipv6CanonicalError — currently defined but not exercised | this folder | 1h |
| Property test ∀a. parse(format(a)) == Ok(a) | this folder | 2h, gated on IPV6CAN-1 to cover the v4-mapped lattice |
| Re-emit-after-canonicalize idempotence (`canonicalize(canonicalize(x)) == canonicalize(x)`) | this folder | 1h |
