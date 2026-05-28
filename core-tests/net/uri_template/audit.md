# `net/uri_template` audit

Module: `core/net/uri_template.vr` (~496 LOC) — RFC 6570 URI
Template expansion (Levels 1-3 in full + Level 4 prefix
`{var:N}` modifier; Level 4 explode `*` partially).

Tests cover the data-surface algebra: TemplateValue 2-variant
construction (Str / List_) + UriTemplateError 2-variant
disjointness. Full functional surface (parse + expand) is
locked-in behind URITPL-1 in `regression_test.vr`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| OpenAPI / HAL / GitHub-API integrations | template substitution. |
| `core.net.weft` route builders | path parameter expansion. |
| AsyncAPI channel-pattern matching | external. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic.

## 3. Language-implementation gaps

### §3.1 URITPL-1 — `UriTemplate.parse` / `expand` SIGSEGV (CLOSED 2026-05-28)

**Pre-fix trigger**: `UriTemplate.parse(&Text)` SIGSEGV'd in LLVM
SmallVector during precompile cascade.

**Closed by source-side fix** (commit `a69f4fd4e`): byte-by-byte
push replaces `extend_from_slice(&b[start..i])` in the literal-
collection loop. Same fix-class as CIDR-1 / HTTPRNG-1 / HTTPCACHE-1
/ CONNEG-1 / LINKHDR-1 — all closed by replacing `extend_from_slice`
intrinsic-chain with explicit byte-by-byte `out.push()` loops in
stdlib parser helpers.

**Post-rebuild validation 2026-05-28**: 7/7 URITPL-1 regression
tests transition from @ignore'd-SIGSEGV to GREEN under `--interp`.

### §3.2 Level 4 explode `*` modifier — partial

Source comment at `uri_template.vr:32` documents that explode
applied to maps is deferred ("map explode lands in a follow-up
when a concrete caller demonstrates the need"). Tests pin only
parse-acceptance of the `*` modifier; expansion semantics
gated by URITPL-1.

## 4. Action items landed in this branch

* `core-tests/net/uri_template/unit_test.vr` — 10 unit tests
  over the data-surface: TemplateValue Str (3) + List_ (3) +
  disjointness (1); UriTemplateError Eq + variant disjointness
  (4).
* `core-tests/net/uri_template/regression_test.vr` — 7
  @ignore'd LOCK-IN pins for URITPL-1: parse literal-only,
  parse simple-var, expand simple-var, parse form-query op,
  parse path-seg op, parse prefix modifier, parse unbalanced
  brace error.
* `core-tests/net/uri_template/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close URITPL-1 (likely batched with CIDR-1 / URL-1) | VBC codegen | 3-5 days |
| Full RFC 6570 Level 1-3 expansion test sweep | this folder | trivial; gated on §3.1 |
| Level 4 explode `*` semantics for List + Map | stdlib + tests | 1 day stdlib + 4h tests |
| Display/Debug round-trip for UriTemplateError | this folder | 30 min |
