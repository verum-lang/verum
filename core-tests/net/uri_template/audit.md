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

### §3.1 URITPL-1 — `UriTemplate.parse` / `expand` SIGSEGV during precompile cascade

**Stable trigger**: any reachable callsite of `UriTemplate.parse(&Text)`
or `UriTemplate.expand(&self, &Map<Text, TemplateValue>)` from a USER
test module SIGSEGVs the compiler inside
`llvm::SmallVectorBase<unsigned long long>::grow_pod`.

Same crash signature family as CIDR-1 + URL-1 — likely all three
share the same root cause in the VBC precompile cascade for stdlib
modules whose functions use one or more of:

* `?` operator on `Result<T, E>` with payload-carrying error variants.
* `text.as_bytes()` → `&[Byte]` view.
* Internal helpers like `Text.from_utf8_unchecked(buf.as_slice())`
  that interact with archive-loader type-id propagation.

**Reproduction**:

```verum
mount core.net.uri_template.{UriTemplate};

@test
fn probe() {
    let s = "/users/{user}".clone();
    let _ = UriTemplate.parse(&s);        // ← SIGSEGV at codegen time
}
```

**Workaround**: none source-side; the precompiled
`runtime.vbca` for `uri_template.vr` has the body compiled
already but the user-side import-cascade re-compile crashes.
Same root as CIDR-1 — investigation should batch.

**Effort**: 3-5 days (root-cause CIDR-1 / URL-1 / URITPL-1 as a
single defect class — likely one VBC codegen edit closes all
three).

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
