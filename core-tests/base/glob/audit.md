# `core/base/glob` — Audit

> Module: `core/base/glob.vr` — POSIX shell-style glob pattern
> matcher. Supports `*` (within-segment wildcard), `**` (cross-segment
> globstar), `?` (single-char), `[abc]` / `[a-z]` (character class),
> `[!abc]` (negated class), `\*` / `\?` (escaped literal).

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `GlobPattern` | record (opaque AST) | yes |
| `GlobError` | sum (parse-time errors) | yes |

### 1.2 Free functions / methods

| Item | Signature |
|---|---|
| `compile` | `(&Text) -> Result<GlobPattern, GlobError>` |
| `compile_with` | `(&Text, Bool) -> Result<GlobPattern, GlobError>` (true = case-sensitive) |
| `matches` | `(&Text, &Text) -> Bool` (compile-and-match in one) |
| `GlobPattern.matches_path` | `(&self, &Text) -> Bool` |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 13 unit tests | all green under `--interp` |
| `property_test.vr` | 5 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 11 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 8 active pins | 8 green |

## §2 — Findings landed in this branch

### 2.1 integration_test.vr referenced a hallucinated API

Pre-fix `integration_test.vr` called:

| Pre-fix call | Status |
|---|---|
| `GlobOptions.new().case_insensitive(true)` | `GlobOptions` type does not exist |
| `compile_with(pattern, opts)` (with `GlobOptions`) | Actual signature is `(&Text, Bool)` |
| `pat = compile("...")` then `pat.matches(path)` | `compile` returns `Result<GlobPattern, _>` — must be unwrapped; method is `.matches_path`, not `.matches` |

Runtime symptom on the `.matches` call:

```
method 'Result.matches' not found on receiver of runtime kind `Result`
... 8 candidate(s) end with `.matches`
(core.base.glob.matches / core.base.semver_constraint.matches /
 Char.matches / AnyChar.matches / CharRange.matches /
 Text.matches / Principal.matches / core.text.char.matches)
```

**Fix in this branch**: rewrote `integration_test.vr` to use either
the free function `matches(pattern, path)` (no compile + unwrap
needed) OR the reusable `compile(pattern).unwrap().matches_path(path)`
form. 11 integration tests now green.

### 2.2 Pre-existing unit and property tests were already correct

`unit_test.vr` and `property_test.vr` used the actual API surface
correctly (free `matches` / `compile.unwrap().matches_path`); only
`integration_test.vr` had the hallucinated API. The unit + property
files are kept as-is.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.glob`:

* `core.io.fs.walk` — filesystem walk path filters.
* `core.cli.*` — argument expansion.
* No other `core/` modules reference this layer.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/glob/integration_test.vr` — rewritten end-to-end
   (11 scenarios across 6 sections):
     §1 Filtering List<Text> via free `matches`
     §2 Negated character classes [!...]
     §3 Escape sequences via compile + matches_path
     §4 compile_with case-sensitivity option
     §5 Literal-only pattern exact match
     §6 Compiled-pattern reuse across paths

2. NEW `core-tests/base/glob/regression_test.vr` — 8 active pins:
     §A `compile_with` takes Bool, not options record
     §B Result must be unwrapped before `.matches_path`
     §C Globstar `**` crosses separators
     §D Star `*` does NOT cross separators
     §E `?` matches exactly one char
     §F Negated class `[!...]` with explicit list + range
     §G Backslash escape for `*` and `?`

3. NEW `core-tests/base/glob/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| GlobError variant disjointness tests | 30min | future task |
| `compile` error-path tests (invalid bracket / unterminated escape) | 30min | future task |
| Property — exhaustive 256-char alphabet for `?` | 1h | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 (alias-unblock landed in semver work this session) |
