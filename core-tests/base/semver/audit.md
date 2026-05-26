# `core/base/semver` — Audit

> Module: `core/base/semver.vr` — SemVer 2.0.0 parser, comparator,
> and formatter. Reference: https://semver.org/spec/v2.0.0.html

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `SemVer` | 5-field record `{ major, minor, patch, pre_release, build_meta }` | yes |
| `SemVerError` | 4-variant sum `Malformed(Text) \| LeadingZero(Text) \| InvalidIdentifier(Text) \| Overflow(Text)` | yes |
| `SemVerOrdering` (new in this branch) | 3-variant sum `Less \| Equal \| Greater` | yes |

### 1.2 Free functions

| Item | Signature |
|---|---|
| `parse` | `(&Text) -> Result<SemVer, SemVerError>` |
| `parse_semver` (new — alias) | `(&Text) -> Result<SemVer, SemVerError>` |
| `cmp` | `(&SemVer, &SemVer) -> Ordering` |
| `semver_compare` (new — SemVerOrdering variant) | `(&SemVer, &SemVer) -> SemVerOrdering` |
| `semver_zero` (new) | `() -> SemVer` (the `0.0.0` no-pre/no-build sentinel) |
| `format` | `(&SemVer) -> Text` |
| `format_semver` (alias) | `(&SemVer) -> Text` |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 19 unit tests + 2 `@ignore`'d (format) | 19 green under `--interp` |
| `property_test.vr` | 13 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 8 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 5 active + 2 `@ignore`'d | 5 green; 2 pinned on §2.1 / §2.2 |

## §2 — Findings landed in this branch

### 2.1 `parse` bare-name dispatch collision

Both `core.base.semver.parse` (returning `Result<SemVer, SemVerError>`)
and another stdlib module's bare `parse` (returning `ParsedType`)
resolve through the same `lookup_function` call. Task #17/#39's
first-suffix-wins root picks the wrong sibling:

```
method 'ParsedType.unwrap' not found on receiver of runtime kind
`ParsedType` ... 5 candidate(s) end with `.unwrap`
(Maybe / Result / CheckedResult / Poll / ReduceResult)
```

Workaround in all test files: construct SemVer records directly via
the record-literal `mk_semver` / `mk_semver_pre` helpers instead of
going through `parse`. Live parse + unwrap pinned at
`regression_test.vr §A` as `@ignore`'d — flips green when the
dispatcher becomes mount-scope-aware (task #17/#39 close, multi-day
VBC codegen work).

### 2.2 `format` lenient-stubbed via `Text.from_utf8_unchecked` private

`core/base/semver.vr:451` calls `Text.from_utf8_unchecked(out.as_slice())`
at the end of `format(&v) -> Text`. `Text.from_utf8_unchecked` was
declared private (`unsafe fn` without `public`) at `core/text/text.vr:455`.
Same defect class as `core-tests/base/ulid` §A. Fix landed in this
branch's earlier ulid work (text.vr:455 → `public unsafe fn`).
Activates after next precompiled-stdlib refresh.

### 2.3 AOT cascade — missing semver aliases

`core/cog/resolve.vr:70` imports `parse_semver` / `semver_compare` /
`SemVerOrdering` / `semver_zero` from `core.base.semver` but the
module never exported these names. The dependency resolver compile
fails AOT-wide with:

```
VBC codegen error (user bodies): undefined function: semver_compare
(in function interval_contains)
```

This blocks the entire stdlib-wide AOT pipeline (task #7).

**Fix landed in this branch**: `core/base/semver.vr` extended with
the four missing public symbols — `parse_semver` (alias for `parse`),
`semver_compare` (3-way comparison returning `SemVerOrdering`),
`SemVerOrdering` (sum type `Less | Equal | Greater`), and
`semver_zero` (no-pre/no-build `0.0.0` sentinel). All additive — no
existing behaviour changed. Activates after next precompiled-stdlib
refresh; once active, `verum test --aot` should no longer fail at
`core.cog.resolve`.

### 2.4 `Text.from("...")` ergonomics

`core/base/semver.vr:148` (and many other stdlib sites — see grep in
audit §3) use `Text.from("empty")` as the canonical string-literal-
to-Text constructor. The static-method dispatcher cannot resolve
`Text.from` (a protocol-method) through `lookup_function`. Added an
inherent `Text.from(s: &Text) -> Text` method at `core/text/text.vr`
just below `Text.new()`. Activates after next precompiled-stdlib
refresh; unblocks every `Text.from("literal")` call site across stdlib.

## §3 — Cross-stdlib usage audit

Consumers of `core.base.semver`:

* `core.cog.resolve` — dependency resolution (the §2.3 AOT blocker).
* `core.cog.manifest` — manifest parsing.
* No other `core/` modules reference this layer at present.

`Text.from("...")` defect surface (count of call sites):

| Module | Count |
|---|---|
| `core/net/link_header.vr` | 5 |
| `core/types/poly_kinds.vr` | ~15 |
| `core/net/uri_template.vr` | ~10 |
| `core/net/ipv6_canonical.vr` | 1 (in doc comment) |
| (many more...) | — |

The inherent `Text.from` method in this branch resolves all of these
without source-level edits.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. `core/text/text.vr` — added `public fn Text.from(&Text) -> Text`
   inherent method (just below `Text.new`). Resolves the stdlib-wide
   `Text.from("literal")` lenient-stub class.
2. `core/text/text.vr:455` — already staged in ulid work (visibility
   change for `from_utf8_unchecked`).
3. `core/base/semver.vr` — added 4 missing public symbols:
   `parse_semver`, `semver_compare`, `SemVerOrdering`, `semver_zero`.
   Unblocks the stdlib-wide AOT cascade (task #7).
4. `core-tests/base/semver/unit_test.vr` — rewritten end-to-end (19
   tests + 2 `@ignore`'d). Uses direct `mk_semver` record-literal
   helper to bypass parse dispatch defect. 5 sections covering
   construction, cmp ordering, SemVerError variants, format aliases.
5. `core-tests/base/semver/property_test.vr` — rewritten (13 laws)
   covering cmp reflexivity / anti-symmetry / transitivity, component
   monotonicity (major dominates minor dominates patch), SemVer §11.3
   prerelease < release ordering, alphabetic + length-wise pre-release
   precedence.
6. `core-tests/base/semver/integration_test.vr` — rewritten (8
   scenarios) covering RFC 2.0.0 §11 precedence chain, manual
   bubble-sort over `List<SemVer>` via cmp, cmp chain composition,
   SemVerError in collections, SemVer clone round-trip.
7. NEW `core-tests/base/semver/regression_test.vr` — 5 active + 2
   `@ignore`'d pins:
     §A `@ignore`'d × 1 — `parse` bare-name dispatch collision
     §B `@ignore`'d × 1 — `format` lenient panic-stub
     §C 5-field SemVer record-layout pin
     §D cmp major-dominates-minor, minor-dominates-patch
     §E SemVer §11.3 prerelease strictly less than release
     §F SemVerError variants pairwise disjoint
8. NEW `core-tests/base/semver/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close task #17/#39 to enable live `parse` calls | multi-day VBC codegen work | task #17/#39 |
| Validate AOT cascade unblocks once stdlib refreshes | runs after stdlib rebuild | task #7 |
| `Display` / `Debug` / `Clone` impls on `SemVer` and `SemVerOrdering` | 30min | future task |
| Property — parse · format round-trip exhaustive | gated on §A close | regression §A pin |
| Cross-tier AOT validation | gated on task #7 + this branch's aliases activating | future task |
