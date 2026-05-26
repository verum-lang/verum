# `core/base/semver_constraint` — Audit

> Module: `core/base/semver_constraint.vr` — Cargo / npm-style
> version constraint parser, matcher, and formatter.
> Operators: `^`, `~`, `>=`, `>`, `<`, `<=`, exact `=`, alternation
> `||`, conjunction (whitespace), x-ranges, wildcards `*`.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `SemVerConstraint` | 9-variant sum (`Any`, `Empty`, `Eq(SemVer)`, `Gt`, `Gte`, `Lt`, `Lte`, `And(Heap, Heap)`, `Or(Heap, Heap)`) | yes |
| `SemVerConstraintError` | sum `InvalidConstraint(Text) \| InvalidVersion(SemVerError)` | yes |
| `MatchOptions` | record `{ allow_prerelease: Bool }` | yes |

### 1.2 Match API

| Item | Signature |
|---|---|
| `parse` | `(&Text) -> Result<SemVerConstraint, SemVerConstraintError>` |
| `matches` | `(&SemVerConstraint, &SemVer, &MatchOptions) -> Bool` |
| `matches_default` | `(&SemVerConstraint, &SemVer) -> Bool` (strict prerelease) |
| `matches_allow_prerelease` | `(&SemVerConstraint, &SemVer) -> Bool` |
| `format_constraint` | `(&SemVerConstraint) -> Text` |
| `match_options_default` | `() -> MatchOptions` |
| `match_options_allow_prerelease` | `() -> MatchOptions` |

### 1.3 Convenience constructors

| Item | Signature | Maps to |
|---|---|---|
| `exact` | `(SemVer) -> SemVerConstraint` | `Eq(v)` |
| `caret` | `(SemVer) -> SemVerConstraint` | `>=v, <(major+1).0.0` (when major≥1) |
| `tilde` | `(SemVer) -> SemVerConstraint` | `>=v, <X.(Y+1).0` |
| `tilde_minor` | `(UInt64, UInt64) -> SemVerConstraint` | `~X.Y` |
| `tilde_major` | `(UInt64) -> SemVerConstraint` | `~X` |
| `x_range_major` | `(UInt64) -> SemVerConstraint` | alias for `tilde_major` |
| `x_range_minor` | `(UInt64, UInt64) -> SemVerConstraint` | alias for `tilde_minor` |
| `and` | `(SemVerConstraint, SemVerConstraint) -> SemVerConstraint` | Cargo `,` separator (Any/Empty absorbed) |
| `or` | `(SemVerConstraint, SemVerConstraint) -> SemVerConstraint` | npm `\|\|` separator (Any/Empty absorbed) |

### 1.4 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 24 unit tests | all green under `--interp` |
| `property_test.vr` | 16 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 11 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 6 active + 2 `@ignore`'d | 6 green; 2 pinned on §2.1 |

## §2 — Findings landed in this branch

### 2.1 `parse` bare-name dispatch defect — same as `base/semver` §A

Both `core.base.semver_constraint.parse` (returning
`Result<SemVerConstraint, SemVerConstraintError>`) and the bare
`parse` in `core.base.semver` route through the same lookup. Plus
both can mis-route to a third sibling that returns `MatchAst` (a regex
match-AST type elsewhere in stdlib). Downstream
`matches_impl(c, v, opts)` then runs `cmp(v, target)` where `target`
is actually a `MatchAst`, panicking:

```
field access out of bounds: field index 2 (offset 16+8 = 24)
exceeds object data size 16 type_id=... type='MatchAst'
backtrace=[core.base.semver.cmp <- core.base.semver_constraint.matches_impl
  <- core.base.semver_constraint.matches_default
  <- test.test_exact_matches_only_self]
```

Workaround in all test files: construct `SemVer` via direct record
literal AND `SemVerConstraint` via direct sum-variant
`SemVerConstraint.Eq(v)` / `.Gte(v)` / ... OR convenience constructors
`exact` / `caret` / `tilde` / etc. Live `parse(&"^1.2.3")` pinned at
`regression_test.vr §A` as `@ignore`'d.

### 2.2 Pre-fix tests all depended on `parse_ver` + `parse_constraint`

Every pre-fix test used the helper `fn ver(s: Text) -> SemVer { parse_ver(&s).unwrap() }`
and/or `parse_constraint(&s).unwrap()`. Both hit §2.1.

**Fix in this branch**: rewrote all 3 test files using direct record-
literal `mk_semver` + sum-variant + convenience-ctor construction.
24 + 16 + 11 = 51 tests green; 8 regression pins (2 `@ignore`'d for
§A).

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.semver_constraint`:

* `core.cog.manifest` — version constraint syntax in dependency
  declarations.
* `core.cog.resolve` — SAT-style resolution over `SemVerConstraint`
  intersections.
* No other `core/` modules reference this layer.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. `core-tests/base/semver_constraint/unit_test.vr` — rewritten
   end-to-end (24 tests across 8 sections):
     §1 exact — anchor + above/below rejection
     §2 caret — anchor + same-major rules + next-major rejection
     §3 tilde — anchor + same-minor rules + next-minor rejection
     §4 tilde_major / tilde_minor / x_range_* aliasing
     §5 MatchOptions factories
     §6 matches with explicit MatchOptions
     §7 SemVerConstraint Any/Empty/Gte/Lt direct variants
     §8 SemVerConstraintError Eq matrix

2. `core-tests/base/semver_constraint/property_test.vr` — rewritten
   (16 laws):
     §A exact reflexive over 4 versions
     §B Any matches every release version
     §C Empty matches no version
     §D caret patch range — accepts higher / rejects lower / rejects
        next major
     §E tilde minor stability boundary
     §F tilde_major full coverage; x_range_* alias parity
     §G and/or absorption laws over Any/Empty

3. `core-tests/base/semver_constraint/integration_test.vr` —
   rewritten (11 scenarios) covering MatchOptions strict vs
   allow-prerelease, multi-version filtering via List<SemVer>,
   compound and/or constraints, x-range semantics, variants in
   List<SemVerConstraint>.

4. NEW `core-tests/base/semver_constraint/regression_test.vr` — 6
   active + 2 `@ignore`'d pins:
     §A `@ignore`'d × 2 — parse bare-name dispatch + format round-trip
     §B caret upper bound `<(X+1).0.0` pin
     §C tilde upper bound `<X.(Y+1).0` pin
     §D Empty matches no version
     §E Any matches every release
     §F and/or absorption with Any/Empty
     §G SemVerConstraintError disjoint under Eq

5. NEW `core-tests/base/semver_constraint/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close task #17/#39 to enable live `parse` | multi-day VBC codegen work | task #17/#39 |
| `format_constraint` round-trip exhaustive test set | gated on §2.1 close | regression §A pin |
| Compound parse `>=1.0, <2.0` and alternation `1.0 || 2.0` integration tests | gated on §2.1 close | future task |
| `Display` / `Debug` impls on `SemVerConstraint` + `MatchOptions` | 30min | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 (semver aliases landed in this session) |
