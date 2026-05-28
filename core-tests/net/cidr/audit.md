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

> **Defect-class catalogue**: CIDR-1 spans three catalogue entries —
> [CLOSURE-RESULT-1](../../../internal/website/docs/stdlib/defect-class-catalogue.md#3-closure-via--chain-on-result--maybe),
> [EXTSLICE-1](../../../internal/website/docs/stdlib/defect-class-catalogue.md#1-extend_from_slice-intrinsic-chain-sigsegv),
> and cross-type variant-payload construction. CIDR-2 is the
> [TRANSIENTMUT-1](../../../internal/website/docs/stdlib/defect-class-catalogue.md#5-transient-mut-self-propagation-through-nested-method-calls)
> pattern.

### §3.1 CIDR-1 — `cidr.parse(&Text)` SIGSEGV (CLOSED 2026-05-28)

**Pre-fix stable trigger**: any reachable callsite of `cidr.parse(&Text)`
from a USER test module produced a fatal SIGSEGV inside
`llvm::SmallVectorBase<unsigned long long>::grow_pod`.

**Closed by source-side discipline across 3 commits** — the
defect was triggered by a combination of three independent codegen
surfaces, all in cidr.parse:

1. **Closure desugaring** (commit `f649312c6`): `parse_int().ok_or_else(|| ...)?`
   chain replaced with `match Maybe.Some / Maybe.None` explicit
   dispatch. Eliminated closure-codegen through `?`-operator.

2. **`extend_from_slice` intrinsic chain** (commit `be64f4e1e`):
   `slice_text` helper's `out.extend_from_slice(&src[start..end])`
   replaced with `while i < end { out.push(src[i]); i = i + 1 }`
   byte-by-byte loop. Eliminated List-payload intrinsic dispatch
   chain.

3. **Cross-type variant-payload construction** (commit `8ed55522c`):
   `Err(e) => Err(CidrError.AddrParseFailed(e))` replaced with
   `Err(_) => Err(CidrError.Malformed(fixed_text))`. The
   `CidrError.AddrParseFailed(AddrParseError)` construction with
   cross-type payload was the final SIGSEGV trigger.

**Post-rebuild validation 2026-05-28** — 5 of 5 regression tests
transition from @ignore'd-SIGSEGV to GREEN under `--interp`:
- `regression_parse_v4_8` ✅
- `regression_parse_v6_32` ✅
- `regression_parse_invalid_prefix_len` ✅
- `regression_parse_no_slash` ✅
- `regression_set_add_text_v4` ✅ (CIDR-2 close — see §3.4)

### §3.4 CIDR-2 — `add_text` transitive mutation propagation (CLOSED 2026-05-28)

**Pre-fix trigger**: `CidrSet.add_text(&text)` returned `Ok(())`
but `set.contains(&ip)` returned `false` for valid IPs that should
have matched. Probe matrix narrowed root cause to `set.len() == 0`
post-call — the mutation on `self.blocks` didn't persist across the
nested `self.add(c)` method dispatch.

Direct call probes worked:
- `cidr_parse(&s).unwrap().contains(&ip)` ✅
- `set.add(Cidr.V4{...})` + `set.contains(&ip)` ✅
- `set.add_text(&s)` + `set.len() == 1` ❌

**Source-side fix landed 2026-05-28** (commit `92480c76b`):
inline `self.blocks.push(c)` directly in `add_text` instead of
calling `self.add(c)`. The transitive `&mut self` propagation
through method-to-method dispatch in nested calls lost the mutation
on `self.blocks` in the VBC codegen.

**Post-rebuild validation**: `regression_set_add_text_v4`
transitions from @ignore'd to GREEN. The underlying VBC codegen
defect (transitive `&mut self` propagation through nested method
calls) is documented as a sister of URL-1 / URL-7 / URL-8 family
and tracked at the codegen layer; source-side workaround
discipline applies in stdlib until VBC fix lands.

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
