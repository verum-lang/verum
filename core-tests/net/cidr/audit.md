# `net/cidr` audit

Module: `core/net/cidr.vr` (~318 LOC) — CIDR notation + IP-range
matcher per RFC 4632 + RFC 4291. Two main types: `Cidr` (typed
v4/v6 prefix + width) and `CidrSet` (List-backed collection
with `contains` / `matching`).

Tests cover the algebraic surface end-to-end through direct
variant construction (`Cidr.V4 { addr, prefix_len: N }`,
`Cidr.V6 { addr, prefix_len: N }`) — `contains` lattice across
prefix widths /0 /8 /24 /32 (v4) and /0 /32 /127 /128 (v6),
cross-family rejection, `num_addresses` saturation arithmetic,
`CidrSet` insertion + lookup, `CidrError` 3-variant Eq.

The parser-path (`cidr.parse(&Text)`) is currently @ignore'd
behind §3.1 — a precompile-cascade SIGSEGV inside LLVM's
SmallVector. Source-side direct-construction tests cover the
runtime data surface; the parser path is locked-in by 5
regression pins in `regression_test.vr` so the defect-shape
does not regress in either direction.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.xds` | Envoy network-filter chain CIDR ACLs. |
| `core.net.weft` | Trust-IP allow-list middleware. |
| Application firewalls / WAF | every per-request IP-classification call. |

## 2. Crate-side hardcodes

None. `core/net/cidr.vr` is pure Verum with no `@intrinsic` bridge.
`mask_equal` is implemented in user-level byte arithmetic.

## 3. Language-implementation gaps

### §3.1 CIDR-1 — `cidr.parse(&Text)` SIGSEGV in LLVM SmallVector

**Stable trigger**: any reachable callsite of `cidr.parse(&Text)`
from a USER test module — including transitive paths through
`CidrSet.add_text(&Text)` — produces a fatal SIGSEGV inside
`llvm::SmallVectorBase<unsigned long long>::grow_pod` during the
precompile cascade for `cidr.vr`. The crash happens during
compilation (before any test body runs).

**Crash signature** (`/Users/taaliman/.verum/crashes/...`):
```
Kind: fatal signal SIGSEGV (11)
Backtrace:
   ...
  11: __ZN4llvm15SmallVectorBaseIyE8grow_podEPvmm
  12: __mh_execute_header
```

**Reproduction** (matches `regression_test.vr` pins):

```verum
mount core.net.cidr.{parse as cidr_parse};
@test
fn probe() {
    let s = "10.0.0.0/8".clone();
    let _ = cidr_parse(&s);     // ← SIGSEGV here at codegen time
}
```

Compare: direct variant construction
`Cidr.V4 { addr: Ipv4Addr.new(10,0,0,0), prefix_len: 8 }`
compiles + executes correctly (see all of `unit_test.vr §1-§9`).

**Likely root cause** (candidates ordered by source-side
suspicion):

1. **Closure in `?`-chain inside `parse`** —
   `parse_int(len_text.as_bytes()).ok_or_else(|| CidrError.Malformed(...))?`
   at `cidr.vr:124-125`. Closure codegen interacting with the
   `?` desugaring for `Maybe → Result` conversion is the most
   likely codegen surface that crashes the LLVM SmallVector.

2. **`text.as_bytes()` from `&Text`** — multiple callsites use
   `text.as_bytes()` to obtain `&[Byte]`. The `&[Byte]` view
   into a `Text` payload may be the trigger if `Text`-layout
   types-by-name propagation through the archive loader hits
   a stale entry. Same defect class as
   [[use_after_free_error_field_shift_2026-05-27]] +
   [[btree_pattern_match_ref_generic_class]].

3. **`@arch_module` annotation interaction with module-import
   precompile cascade** — `cidr.vr` ships under `@arch_module(
   foundation: Foundation.ZfcTwoInacc, stratum:
   MsfsStratum.LFnd, lifecycle: Lifecycle.Theorem("v0.1"))`.
   Other modules with this annotation (e.g. `core.net.addr`,
   `core.net.url`) DO compile under user tests, so this is
   the *least* likely candidate.

**Fix path**: multi-day VBC + codegen work. The closure
desugaring in #1 should be probed first by replacing
`ok_or_else(|| ...)` with a plain `match` — if the crash
disappears, the closure path is the surface. Source-side
workaround is straightforward (the closure here is a
single-argument constant constructor) but does not close the
underlying defect class.

**Effort**: 1 day to diagnose root cause + 2-3 days fix VBC
codegen + retest.

### §3.2 `Cidr.contains` slice-deref pattern

`contains(&self, ip: &IpAddr)` uses `[Byte; 4]` literal
construction + `&a[..]` slice deref:

```verum
let a: [Byte; 4] = [a1, a2, a3, a4];
let b: [Byte; 4] = [b1, b2, b3, b4];
mask_equal(&a[..], &b[..], *prefix_len)
```

This shape works under both `--interp` and as exercised by
the conformance suite, so no defect surfaced — pinning for
future-codegen safety.

### §3.3 `clone_cidr` workaround for variant payload cloning

The internal `clone_cidr` free function at `cidr.vr:244-251`
re-constructs the variant manually instead of using
`(*c).clone()`. Source-side comment doesn't explain why; the
likely reason is the same payload-clone codegen hazard
documented in
[[btree_pattern_match_ref_generic_class]] under "variant tag
mis-read for &Maybe<Heap<RecordWithGenericParams>>".

The conformance suite exercises this through `CidrSet.matching`
which clones a stored `Cidr` via `clone_cidr` to return as
`Maybe<Cidr>`. Tests pass, so the workaround is durable.

## 4. Action items landed in this branch

* `core-tests/net/cidr/unit_test.vr` — 35 unit tests covering
  direct `Cidr.V4`/`V6` variant construction, `contains`
  lattice (v4 /0 /8 /24 /32, v6 /0 /32 /127 /128, cross-family
  rejection), `num_addresses` (v4 1/2/256/65536 + v6 1/2 +
  /32-v6 saturation to UInt64.MAX), `CidrSet` insertion +
  contains + matching (Some/None), `CidrError` 3-variant Eq
  (InvalidPrefixLen + AddrParseFailed + variant disjointness).
* `core-tests/net/cidr/regression_test.vr` — 5 @ignore'd
  LOCK-IN pins for CIDR-1: parse-v4, parse-v6, set.add_text,
  parse-invalid-prefix-len, parse-no-slash.
* `core-tests/net/cidr/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close CIDR-1 (closure in `?`-chain inside parse) | VBC codegen | 3-5 days incl rebuild |
| `cidr.parse` happy + error path coverage | this folder | trivial; gated on §3.1 |
| `format(&Cidr) -> Text` round-trip | this folder + stdlib add | 2h after Display impl lands |
| `Cidr.network_address` / `broadcast_address` derivation | this folder + stdlib add | 4h |
| `CidrSet` longest-prefix-match (currently first-match O(N)) | stdlib + tests | 1 day for radix-trie ordering |
