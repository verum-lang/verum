# Verum testing — capability gaps

> Findings discovered while building `core-tests/`. Each item is a
> concrete fundamental improvement to make Verum's testing story
> reference-class. Severity ranks the impact on developer productivity.
> Each item names files to change and the architectural principle
> involved — these are roadmap items, not patches.

## §0  Existing capabilities (baseline)

| Technique | Marker | Where |
|---|---|---|
| Unit tests | `@test` | `verum test` discovers AST attribute |
| Parametrised | `@test_case(args)` | `crates/verum_cli/src/commands/test.rs::parse_test_cases` |
| Property-based with shrinking | `@property` | `crates/verum_cli/src/commands/property.rs` (Hedgehog-style integrated rose-tree shrinking, seed replay) |
| Skip markers | `@ignore` / `@ignored` | discover_tests in test.rs:1564 |
| Cross-tier execution | `--interp` / `--aot` | test.rs::TestOptions::tier |
| Replay from regression DB | (automatic) | property.rs writes `target/test/pbt-regressions.json`; replay-first on subsequent runs |
| Stable RNG | `@property(seed = 0x...)` | property.rs::parse_property_attr_args |

## §1  Critical gaps (impact developer productivity directly)

### 1.1  Property generators only support primitives

**File:** `crates/verum_cli/src/commands/property.rs:158-175` — `Generator::for_type()`
matches only `TypeKind::Bool`, `TypeKind::Int`, `TypeKind::Float`, `TypeKind::Text`,
plus refined `Int{x : x > 0}` style.

**Missing:** generators for compound stdlib types:

| Type | Generator should produce |
|---|---|
| `Maybe<T>` where `T` has generator | 50% None, 50% Some(gen<T>()) |
| `Result<T, E>` where both have generators | 50% Ok, 50% Err |
| `List<T>` where T has generator | length-shrinkable random list |
| `Map<K, V>` where K: Hash + Eq, V: any | random key-value pairs |
| `Set<T>` | random distinct elements |
| `Ordering` | uniform over {Less, Equal, Greater} |
| Tuples `(A, B, C)` | product of generators |
| User enums (any closed-variant `type T is A | B(X)`) | uniform-over-variants with payload generator dispatch |
| User records | field-wise compositional generator |

**Why critical:** `@property fn prop_xxx(opt: Maybe<Int>)` should Just Work.
Right now it silently falls through to a default and the test runs once with
a default value, defeating the property semantics.

**Recommendation:** add a `Generator::for_type` recursive builder that:
1. Resolves user types via the metadata registry (variants → enum gen).
2. Composes generators structurally for tuples / records / generics.
3. Falls back to a `Default::default()`-style generator only when no
   structural recipe is available — and **emits a warning** that the test
   ran without sampling.

**Scope:** ~400 LOC in `property.rs`; cross-references metadata loading.

### 1.2  Property-tests don't exist for higher-kinded laws

A property like *"map over a Maybe is a functor"*:
```verum
prop_maybe_map_identity: ∀ m: Maybe<T>. m.map(|x| x) == m
```

cannot be expressed today because (a) no generator for `Maybe<T>` (§1.1),
and (b) generic-quantified `T` in the property signature is not supported
by the runner's monomorphisation.

**Recommendation:** lift `@property` to support generic parameters with
default instantiation policy: each generic var instantiates as `Int` for
the primary run plus alternative tier-runs at `Bool`, `Text`, `Float`.

**Scope:** schema change in `PropertyFunc`; ~150 LOC in property.rs.

### 1.3  No test fixtures (`@setup`, `@teardown`)

For tests that need shared state (database connection, temp directory,
context-system registration), the only options today are:

- Repeat setup in every `@test` body (brittle, slow).
- Lazy-init via `LazyCell`-static — leaks state across tests, breaks
  isolation.

**Recommendation:** add `@setup` and `@teardown` attributes:
```verum
@setup
fn make_db() -> TestDb { TestDb.in_memory() }

@teardown
fn drop_db(db: TestDb) { db.close() }

@test
fn test_query(db: TestDb) {
    db.insert("k", "v");
    assert_eq(db.get("k"), Some("v"));
}
```

The runner injects setup result into the test signature. Per-test or per-suite
scoping declared by attribute argument: `@setup(scope = "suite")`.

**Scope:** AST attribute extension + runner state-management; ~300 LOC.

### 1.4  No test isolation between runs

`crates/verum_compiler` keeps a global `FileId` counter and a cached
stdlib registry. Two tests compiled in the same process pollute each
other's namespace. The legacy `vcs/runner/vtest::isolation.rs` enum exists
but has no implementation.

**Symptom:** parallel test runs produce non-deterministic failures
when one test mutates a global context-system entry that another test
inspects. The user has no way to opt into "fresh process per test".

**Recommendation:** add `@isolated` attribute for per-test process isolation
(default for tests using contexts), and add a runner option to enforce
process-per-test globally for L0 conformance.

**Scope:** runner-orchestration changes in test.rs; ~200 LOC.

### 1.5  `--coverage` flag exists but no implementation

`TestOptions::coverage` flag is parsed (`test.rs:74`) and read into
`TestRunCfg::coverage` but no instrumentation pass is wired. CI cannot
report coverage today.

**Recommendation:** instrument VBC bytecode at compile time with
basic-block hit counters; emit lcov-format on test exit. Consult
LLVM source-based coverage where AOT.

**Scope:** new `verum_codegen/src/coverage.rs`; ~600 LOC + lcov writer.

## §2  Medium gaps (workflow ergonomics)

### 2.1  No snapshot / golden-file testing

For tests that compare large textual output (Display formatter,
serialisation, codegen IR), there's no `assert_snapshot!`-style macro
with auto-update.

**Recommendation:** `@snapshot` attribute that captures stdout (or a
named buffer) and compares to a sibling `.snap` file. With
`--update-snapshots` the file is overwritten; without it, mismatches
fail with a unified diff.

### 2.2  No mocking framework

Stubbing a context-system value or replacing a free function for the
duration of a test requires hand-rolling. No `Mock<T>` or `expect!`
macro analogue.

**Recommendation:** integrate with the context system: `@mock(Logger)`
parameter injects a recording mock and exposes `mock.calls()` for
post-condition assertions.

### 2.3  No time / clock injection

Tests that depend on `Instant.now()` / `SystemTime.now()` cannot freeze
the clock without context-system gymnastics.

**Recommendation:** standard `TestClock` context that replaces wall-clock
in test mode by default.

### 2.4  No fuzzing harness for `@test`

`vfuzz` exists separately (`vcs/Makefile:bench-fuzz`) but isn't
auto-discovered alongside `verum test`. A `@fuzz` attribute that runs
the same body with libFuzzer-style coverage-guided mutation would let
authors evolve test corpora alongside their tests.

**Recommendation:** wire vfuzz into `verum test --fuzz`; default budget
60s per `@fuzz`.

### 2.5  No `assert_eq` diff formatter

When `assert_eq(a, b)` fails on a large value (a 100-element list, a
deeply nested record), the failure message is the raw stringified
values with no structural diff.

**Recommendation:** richer assert diagnostics — colourised structural
diff, line/field-level highlighting.

## §3  Architectural gaps (testing infrastructure substrate)

### 3.1  Differential testing is per-tier-run, not per-tier-result

`verum test` runs each test under whichever tier you select. There is
no built-in **cross-tier divergence detection**: running once with
`--interp` and once with `--aot` and comparing exit code + stdout +
stderr + panic message would catch tier-mismatch bugs (kernel-soundness
incidents).

**Recommendation:** `verum test --differential` that runs each test
under both tiers and fails if they disagree. Exit-code / stdout / stderr
must match byte-for-byte. This is the structural fix for the
*"Differential testing aspirational"* finding in the original
infrastructure audit.

**Scope:** runner orchestration; ~200 LOC.

### 3.2  No structured error-code registry

`@expected-error: E999` (vtest legacy) accepts any string. Verum
has no machine-readable registry of which compiler phase emits which
error code, so:

- typos in expected-error directives silently pass
- error-code categories drift between phases without a single source

**Recommendation:** add `crates/verum_error/src/registry.rs` keyed by
`E\d{3}` with phase, category, and message template. The diagnostic
emission API consults the registry; CI fails on unknown codes.

### 3.3  Refinement contracts are not auto-tested

A `where`-clause on a function (`fn f(x: Int) where x > 0`) is verified
by SMT at typecheck time. There's no automatic test-fuzzing-the-boundary
mechanism: someone could write `@test fn t() { f(0) }` and the SMT
verifier would *correctly reject*, but the developer experience would
be richer if `verum test` *generated* boundary-violation cases from the
contract.

**Recommendation:** at compile time, emit a `@property` companion for
each refinement contract that samples both satisfying and violating
inputs and asserts the expected verdict.

### 3.4  Test metadata is single-dimensional

Tests can be tagged via `@test_case` arguments, but there's no
`@flaky`, `@slow`, `@hardware(gpu)`, `@deprecated` taxonomy.
Operationally:
- flaky tests can't be auto-retried 3× before failure
- slow tests can't be auto-timeout-extended
- hardware-dependent tests can't be auto-skipped on CI without GPU

**Recommendation:** standardise these as well-known test attributes.
Runner consults each on dispatch.

### 3.5  No CI-format-change advisory

When `--format json` output changes (a new field, a renamed event), CI
consumers downstream break silently. There's no schema versioning.

**Recommendation:** include `"schema_version"` in every JSON event;
runner emits a warning when consumer-side schema is older.

## §4  How to use this document

When working on a `core-tests/<...>/<module>/` test suite and you hit a
testing-infrastructure rough edge:

1. Find the gap above (or add a new entry — keep §1/§2/§3 ranking
   discipline). Note the audit-source as `core-tests/<module>/audit.md`.
2. Do **not** detour to fix it inline — it's almost always
   cross-cutting. Log in this file under the matching severity.
3. Before claiming a new module, scan §1 for high-leverage items and
   consider whether a fundamental fix would unblock multiple modules.
4. When a §1 item lands, mark it crossed-out (`~~text~~`) here, with
   the commit hash and the date — keep institutional memory of what
   was deferred and when it came home.

The goal is not to keep this list growing indefinitely. The goal is
that this document is empty.
