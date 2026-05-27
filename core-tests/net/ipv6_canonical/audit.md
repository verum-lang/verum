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

### §3.1 IPV6CAN-1 — `format_v4_mapped` precompile SIGSEGV

**Stable trigger**: calling `format_ipv6(&a)` where `a` is
v4-mapped (any address whose first 5 segments are 0 and
segment[5] == 0xFFFF). The IPv4-mapped branch invokes
`format_v4_mapped`, which SIGSEGVs during the precompile
cascade for `ipv6_canonical.vr`.

**Crash signature** matches CIDR-1 — SIGSEGV inside
`llvm::SmallVectorBase<unsigned long long>::grow_pod` during
codegen of the v4-mapped emit-path. The non-v4-mapped branch
(`format_ipv6` main body) compiles and executes correctly,
covering >95% of canonical-form semantic surface.

**Reproduction**:

```verum
mount core.net.ipv6_canonical.{format_ipv6};
mount core.net.addr.{Ipv6Addr};

@test
fn probe() {
    let a = Ipv6Addr.new(0, 0, 0, 0, 0, 0xffff, 0, 0);  // ::ffff:0.0.0.0
    let s = format_ipv6(&a);                              // ← SIGSEGV at codegen
}
```

**Likely root cause** (candidates ordered by source-side
suspicion):

1. **Byte-string literal `b"::ffff:"` in `push_bytes` call** —
   `format_v4_mapped` at `ipv6_canonical.vr:195` uses
   `push_bytes(&mut out, b"::ffff:")`. Byte-string literals
   may not have full codegen coverage in the VBC precompile
   cascade for stdlib modules called from user tests.

2. **High-index octet access through array-of-byte parameter** —
   `format_v4_mapped(_: &[UInt16; 8], octets: &[Byte; 16])`
   accesses `octets[12]`, `octets[13]`, `octets[14]`,
   `octets[15]`. Constant-index accesses past the conventional
   first-8-byte run may hit a codegen edge in array layout
   propagation. Same defect class family as
   [[btree_pattern_match_ref_generic_class]] +
   [[enactment_field_access_oob_2026-05-24]].

3. **`push_decimal_byte` multi-branch arithmetic** —
   3-branch conditional with `v / 100`, `v / 10`, `v % 10`.
   Combined with byte casts could trigger codegen of a
   branching path that interacts with overflow-check
   instrumentation.

**Fix path**: 1-day diagnosis to isolate which of the three
candidates is the trigger, then VBC codegen edit + rebuild.
Source-side workaround would be to inline the v4-mapped logic
without the byte-string literal and pass the high-index octets
as separate Byte parameters — would defer the underlying defect
class.

**Effort**: 1 day to diagnose + 2-3 days fix + retest.

### §3.2 `canonicalize` cascades to `Ipv6Addr.parse` workarounds

`canonicalize` calls `parse` which delegates to `Ipv6Addr.parse`
— the latter has documented VBC codegen workarounds at
`addr.vr:760-808` (see `net/addr/audit.md §3.2`). The
conformance suite exercises the working canonical path.

## 4. Action items landed in this branch

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
