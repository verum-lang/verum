# `context/standard` audit

Module: `core/context/standard.vr` (~700 LOC) — the canonical set of
DI context types: 10 `context` protocols (Logger, Database, Auth,
Config, Cache, Metrics, Tracer, Clock, Random, FileSystem) plus the
supporting data types (ContextLogLevel, ContextLogRecord, Row,
QueryResult, AuthUser).

Tests: `unit_test.vr` covers all data types (ContextLogLevel ADT
with severity/name/is_enabled + Eq/Ord/Clone/Copy/Debug/Display,
AuthUser.has_role, Row.get_index, QueryResult Display).

## 1. Cross-stdlib usage

`ContextLogLevel`:
* `core/base/log.vr` defines a SEPARATE `LogLevel` ADT (`Trace |
  Debug | Info | Warn | Error`). These two types co-exist — the
  `LogLevel` is the lower-level base/log integration; the
  `ContextLogLevel` is the context-system DI surface (includes
  `Fatal` as a 6th variant).

`Row`, `QueryResult`:
* Mirror the row/result types in `core/database/common/*` (with
  potentially different field layouts). The context-system `Row`
  is the test-time seam; production code uses the database-module
  types directly.

`AuthUser`:
* Used as the return type of `Auth.current_user() -> Maybe<AuthUser>`.
  No other consumer in `core/`.

The 10 context protocols themselves are user-facing: applications
provide concrete implementations at startup.

## 2. Crate-side hardcodes

`crates/verum_compiler/src/phases/context_check.rs` recognises
these context type names as the "well-known" set and assigns them
the compile-time slot IDs (0..N range) used by `env_ctx_get` /
`env_ctx_set` for the O(1) slot fast-path. Drift here is caught
by integration tests of the runtime ctx_bridge.

The `Tracer` context's `Span` type is `core.tracing.span.Span`
(NOT a parallel definition in standard.vr); this is documented at
the top of the Tracer section in `standard.vr` and verified at
compile time by cross-module name resolution.

## 3. Language-implementation gaps

### §3.1 Two `LogLevel` types (`base/log.LogLevel` vs `context/standard.ContextLogLevel`)

The duplication is intentional (different audiences, different
variant sets — Fatal is in Context but not Base) but easily
confusing. `unit_test.vr` consistently uses `ContextLogLevel` to
dodge the bare-name collision. Document the distinction in
`standard.vr` doc-comment and in the website docs.

**Effort:** trivial (~10 min doc edit).

### §3.2 `Row.get(name: &Text) -> Maybe<&Text>` linear scan

```verum
public fn get(&self, column: &Text) -> Maybe<&Text> {
    for i in 0..self.columns.len() {
        if &self.columns[i] == column {
            return self.values[i].as_ref();
        }
    }
    None
}
```

Linear scan is fine for tens of columns but quadratic for joins
with hundreds of result columns. Acceptable for the test-seam
contract, but the production database `Row` types should use a
hash-indexed column lookup. Document the trade-off; add a
`Row.get_indexed(idx: Int) -> Maybe<&Text>` accessor (already
exists as `get_index`) for callers that pre-resolved the index.

**Effort:** documentation only.

### §3.3 Context protocols cannot be unit-tested in pure Verum

The `context Logger {}` / `context Database {}` etc. declarations
require compiler support for `provide` / `using` to instantiate.
Without that, `unit_test.vr` here is restricted to the DATA TYPES
that back the protocols. The protocols themselves are tested at
the language level (in `vcs/specs/L2-standard/contexts/`), not
in this folder. Document this clearly in `core-tests/README.md`.

**Effort:** ~30 min doc + cross-reference.

### §3.4 No `ContextLogLevel.from_severity(Int) -> Maybe<ContextLogLevel>`

The forward map `severity()` is one-way today. Adding
`from_severity(n)` makes `ContextLogLevel` round-trippable via
its ordinal — useful for serialization (e.g. Prometheus log-level
labels). Add it and pin the round-trip law in property_test.vr.

**Effort:** ~30 min impl + test.

### §3.5 Archive-loaded `Row` — two facets (NEW, HIGH)

**Facet #1 — `Row.get_index(n)` / `Row.get(&name)` method-return of
`Maybe<&Text>` — CLOSED 2026-06-03.** Calling these and consuming the
result used to SIGSEGV the compiler during execution-compile (both return
`Maybe<&Text>` borrowed from `self.values[i].as_ref()`). It is **closed**
by codegen work that landed between 2026-06-01 and `a1293ff52` (the
record-allocation / `Self {}`-literal / transparent-ref family — same
class as [[btree_pattern_match_ref_generic_class]] / CLASS-9 / D2).
Validated GREEN on a build of that codegen: the `get_index(0) is
Maybe.Some` regression pin **and** the three `unit_test.vr` `get_index`
tests (which `match` + dereference the `&Text` payload, `assert_eq(*s,
…)`) all pass. All four were **un-`@ignore`'d**.

**Facet #2 — reading `Row`'s OWN fields (`r.columns` / `r.values`) from
USER code — CLOSED 2026-06-04.** A DISTINCT defect (clean field-OOB panic,
not a SIGSEGV): `field index 4 (offset 40) exceeds object data size 16
type='Row'`. ROOT CAUSE (traced with a `[RECTYPE]` codegen instrument):
the bare name `Row` is a **3-way collision** — a `Row` PROTOCOL, the
`core.context.standard.Row` RECORD, and the variant
`StepResult.Row(reg_start, n_cols)` (`core.database.sqlite.native.l4_vdbe`).
`let r = Row { columns, values }` recorded `variable_type_names["r"] =
"StepResult"` because, at `compile_let`'s `extract_expr_type_name` call,
the record `Row` is **not yet loaded** (lazily loaded only during
`compile_record`): `type_name_to_id["Row"]` first-wins to the PROTOCOL,
`type_field_layouts["Row"]` is absent, no Record descriptor named `Row`
exists in `self.types` — so only `find_variant_parent_type_by_args("Row",
2)` resolves, and it matches `StepResult.Row` by ARG COUNT alone (2 == 2)
→ `"StepResult"`. Every later `r.<field>` read then resolved against
StepResult's absent layout → a GLOBAL field-intern index (4) → OOB.

**FIX:** `extract_expr_type_name` now FIELD-NAME-verifies the
`find_variant_parent_type_by_args` result — it accepts the variant parent
only when the literal's field NAMES match the variant's declared fields
(via `find_variant_in_type_descriptors`, whose parent IS loaded). For
`Row { columns, values }` vs `StepResult.Row(reg_start, n_cols)` the names
differ → the variant match is rejected → the literal resolves to its own
record type `Row`. Independent of the record's lazy-load state, and
regression-safe (legitimate bare record-variant literals still match by
name; arg-count-only collisions fall through).

Pinned: `regression_row_direct_field_read_after_collision`
(`regression_test.vr`) + the simplified `mod/unit_test.vr` umbrella test
(now reads `r.columns`/`r.values` directly). Validated: standard 74/0/0
(incl. the new pin), error 56/0/0, scope 62/0/0.

### §3.6 `f"{Type.Variant}"` does not dispatch `Display` — **CLOSED 2026-06-01**

A DIRECT variant-constructor in an interpolation placeholder
(`f"{ContextLogLevel.Info}"`) rendered the variant name (`"Info"`) instead
of the `Display` output (`"INFO"`); only the bound-var form dispatched.
**Closed by commit `19bb51b3a`** (`fix(vbc/codegen): … qualified-variant
Display`): `infer_expr_type_name` now recognises `Field{Path(Type),
Variant}` and returns `<Type>` when it declares that variant, so the
interpolation routes through `<Type>.fmt`. **Validated on a clean worktree
build of HEAD `f64d7e4fc`** (which includes `19bb51b3a`):
`f"{ContextLogLevel.Trace}"` → `"TRACE"` etc. GREEN. The pin
`regression_display_direct_ctor_renders_uppercase_name` was un-`@ignore`'d
and kept as a re-regression guard. Tracked as
[[fstring_direct_variant_ctor_display_dispatch]].

NOTE — a SEPARATE, still-open case: `f"{err}"` for a *record-variant* ADT
(`ContextError`) via a **bound var** still renders the default
`NotFound(...)` instead of `Display`→`message()` (gate-detection, not the
direct-ctor inference path). `19bb51b3a` does NOT close it; the 5 error
Display pins (error/{unit,property,integration}) stay `@ignore`'d — see
`context/error/audit.md §3.4`.

### §3.7 Row field-shift on cross-module direct field read (NEW, part of CLASS-9)

Reading `Row`'s own fields (`r.columns` / `r.values`) from USER code (e.g.
through `mount core.context.*`) panics `field access out of bounds: field
index 4 ... type='List'` — the archive-loaded record's field index is
mis-resolved. `AuthUser` fields (`u.id`/`u.name`/`u.roles`) and
`QueryResult.rows` read correctly, so the shift is type/layout-specific to
`Row`. `mod/unit_test.vr`'s Row test was reworked to read only
`qr.rows.len()` (the working path). Same CLASS-9 family as §3.5.

## Conformance status (2026-06-01, interpreter / `--test-threads 1`)

`ContextLogLevel` (severity / name / is_enabled / Eq / Ord / Clone / Debug
/ Display-via-bound-var), `AuthUser` (has_role / Display), `QueryResult`
(construction / Display / `.rows`) are GREEN. Blocked + pinned: `Row.get`
/ `Row.get_index` / `Row` direct field read / `f"{Type.Variant}"` Display.
Status: **partial** (was `regression-only`).

## Action items landed in this branch

* `core-tests/context/standard/unit_test.vr` — surface coverage for
  ContextLogLevel + data types; `get_index` tests `@ignore`'d (§3.5),
  Display test rewritten to bound-var idiom (§3.6).
* `core-tests/context/standard/regression_test.vr` (NEW) — pins §3.5,
  §3.6 with minimal repros + live working-idiom companions.
* `core-tests/context/standard/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Document LogLevel vs ContextLogLevel distinction | `core/context/standard.vr` doc + website | 10 min |
| Add `Row` linear-scan note + `get_index` recommendation | `standard.vr` doc | 10 min |
| Add `ContextLogLevel.from_severity(Int) -> Maybe<...>` + round-trip property | `standard.vr` + tests | 30 min | **LANDED** (commit `4c9acaa5a`; 4 unit + 2 property GREEN incl. severity∘from_severity round-trip) |
| Write property/integration/regression tests for ContextLogLevel | this folder | 1h |
| Cross-tier validate context protocols (requires `provide`/`using`-aware test harness) | language-level vcs specs | tracked elsewhere |
