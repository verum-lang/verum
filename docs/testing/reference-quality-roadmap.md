# Reference-quality Testing for Verum — Research & Roadmap

**Scope**: the developer-facing testing experience, reached through
`verum test`, `verum coverage`, and the stdlib attributes (`@test`,
`@property`, `@test_case`, `@snapshot`, `@before_each`, `@after_each`,
`@ignore`). This document covers **application-level testing** that
Verum developers write. It deliberately excludes the compiler's own
conformance suite (`vtest` / `vbench` at `vcs/runner/`), which serves
a different audience and stays as-is.

## 1. State of the art — survey

| Framework | Language | Differentiator | What Verum should borrow |
|-----------|----------|----------------|--------------------------|
| QuickCheck (Hughes & Claessen, 1999) | Haskell | First PBT; `Arbitrary` typeclass; separate `shrink :: a -> [a]` | Random testing principle; shrinking as a first-class obligation |
| Hypothesis (Python, MacIver) | Python | Explicit strategies; **regression database** (`.hypothesis/examples/`); coverage-guided mutation; `@settings` | Regression DB; user-visible replay seed on failure; example DB persistence |
| Hedgehog (Haskell/F#) | Haskell | **Integrated shrinking**: generator emits a lazy rose-tree of `(value, shrinks)` — no separate Shrink instance | Integrated tree avoids the whole "shrinker got out of sync with the generator" class of bugs |
| PropTest (Rust) | Rust | Hypothesis-style in Rust; explicit Strategy trait; file-backed regression DB | Same as Hypothesis but idiomatic in a typed language |
| FastCheck (JS) | JavaScript | Arbitrary trees, biased generators, path replay via string | String-encoded replay for CI logs — one line tells you how to reproduce |
| CrowBar (OCaml, Dolan) | OCaml | PBT driven by afl-fuzz — coverage-guided input search | Coverage-guided generation once the infra is in place |
| SmallCheck | Haskell | Enumerates all values up to a depth bound — **exhaustive** for small inputs, catches corner cases random misses | Hybrid mode: `SmallCheck-for-tiny, QuickCheck-for-the-rest` |
| Kitty / Targeted PBT (Löscher & Sagonas) | Erlang | Search-guided: score each input against a user-supplied "how close to interesting" metric; simulated annealing toward failures | Powerful for state-space exploration; deferred to Stage 3 |
| Model-based testing (FsCheck, ScalaCheck) | F#/Scala | User writes a reference model; the framework drives random sequences of ops and diff-checks against the model | Maps onto Verum protocols: `impl RealThing for T` + `impl Model for T` checked for equivalence |
| Doctest (Python / Rust / Haskell) | Various | Examples inside doc comments **are** tests; one source of truth for both docs and correctness | Verum `/// @example` blocks compile-and-run during `verum test` |
| Snapshot / golden (Jest, insta, syrupy) | Various | Serialised output stored alongside test; diff against it on rerun; `--update` regenerates | First-class `@snapshot(name)` + `target/snapshots/*.snap` |

### Takeaways for Verum

Three design invariants worth locking in:

1. **Integrated shrinking (Hedgehog)** over separate `Arbitrary/Shrink`
   pairs. Refinement types make this especially cheap — a generator
   for `Int{ 0 < it < 100 }` is a single thing that both produces and
   shrinks within the bound, no chance of shrinking to 0 and failing
   the refinement after the fact.

2. **Seed-based determinism** with an on-disk regression DB. Every
   failure prints a one-line replay command:
   `verum test --property-seed 0x1a2b3c4d --filter my::prop`. The DB
   at `target/test/pbt-regressions.json` stores those seeds and is
   replayed first on every subsequent run — a CI flake becomes a
   permanent regression after one hit.

3. **Refinement-driven generators for free**. Verum's refinement types
   (`Int{ it > 0 }`, `Text{ it.len() <= 32 }`) describe valid domains
   declaratively. The PBT runner **reads the refinement** and
   generates-within-bound directly, so:

   ```verum
   @property
   fn commutative(x: Int{ 0 < it < 1000 }, y: Int{ 0 < it < 1000 }) {
       assert_eq(x + y, y + x);
   }
   ```

   …requires zero boilerplate. This is the single biggest win Verum's
   type system earns us over PBT in plain Rust / Haskell.

## 2. Design for Verum

### 2.1 Runner architecture

The runner lives **Rust-side**, inside `verum_cli`'s test command. It
reflects on the `@property` function's parameter types via the parsed
AST, synthesises generators for each parameter, and drives the VBC
interpreter directly:

```
  discover @property  →  read param types from AST
                      →  build Generator<T> per param (Rust-side)
                      →  loop N times:
                           ├─ sample inputs with a fresh seed
                           ├─ encode to VBC Values
                           ├─ interpreter.execute_function(prop_id, args)
                           └─ catch InterpreterError::Panic / AssertionFailed
                      →  on failure: shrink via Hedgehog tree
                      →  store (prop_name, seed) to regression DB
                      →  print counterexample + replay command
```

Why Rust-side and not pure-Verum? Two reasons:

1. **Bootstrap economics** — writing generators in Verum requires
   first-class lazy lists, polymorphic traits, and RNG. We'd ship
   year-two before delivering Stage-1 PBT. Rust gets us end-to-end
   value today; a pure-Verum port is feasible later.

2. **Integrator advantage** — the runner already owns the VBC module,
   the interpreter, the AST, the refinement-type info, and the RNG.
   Verum-side PBT would have to FFI back to most of this.

### 2.2 Generator primitives (Stage 1)

| Type | Generator | Shrink strategy |
|------|-----------|-----------------|
| `Bool` | fair coin | shrink to `false` |
| `Int` | biased toward 0, then exponential outward, edge cases `{0, 1, -1, i64::MIN, i64::MAX}` first | halving toward 0 |
| `Int{lo..hi}` | uniform in bound | halving toward nearest edge inside bound |
| `Nat` | `Int{ it >= 0 }` | halving toward 0 |
| `Float` | uniform over representable, edge cases `{0.0, -0.0, ∞, -∞, NaN, subnormal, 1.0, -1.0}` | halving |
| `Text` | length ∈ [0..size], ASCII biased but emits UTF-8 scalars including BMP + supplementary planes | remove chars, then simplify each char |
| `List<T>` | length ∈ [0..size] of generator for `T` | shrink length then shrink elements |
| `Option<T>` | 1/4 None, 3/4 Some(T) | prefer None, else shrink inner |
| `(T, U, V)` | tuple of sub-generators | shrink element-wise |
| `@derive` record | struct with generated fields | shrink field-wise |
| Sum type | uniform over variants, shrink to smallest variant | constructor-wise |

### 2.3 Attribute surface

```verum
@property
fn roundtrip(x: Int) {
    assert_eq(decode(encode(x)), x);
}

@property(runs = 10_000, seed = 0x1234)
fn thorough(bytes: List<Byte>{ it.len() <= 1024 }) {
    let decoded = parse_frame(&bytes);
    assert(decoded.is_ok() || bytes.len() < 4);
}

@test_case(0, 0, 0)
@test_case(1, 2, 3)
@test_case(-5, 5, 0)
fn add(a: Int, b: Int, expected: Int) {
    assert_eq(a + b, expected);
}

@before_each
fn setup() {
    // runs before every @test / @property in this module
}

@after_each
fn teardown() { }

@snapshot
fn render_greeting() -> Text {
    format("hello, {}!", "world")
}

@test
@ignore(reason = "flaky on CI #1234")
fn needs_network() { ... }
```

### 2.4 Regression database

`target/test/pbt-regressions.json`:

```json
{
  "schema": "verum-pbt-regressions/v1",
  "entries": [
    {
      "test": "my_module::commutative",
      "seed": "0x1a2b3c4d",
      "first_seen": "2026-04-24T20:15:02Z",
      "shrunk_input": "(42, 17)",
      "last_replayed_at": "2026-04-24T20:16:11Z"
    }
  ]
}
```

Every run replays these seeds **before** drawing fresh ones. Fixing
a bug drops the entry (the test passes on the stored seed); the
author removes it manually or `--property-clear-regressions` prunes.

### 2.5 Coverage integration

`verum coverage`:

1. Runs `verum test --coverage` (sets LLVM instrumentation flags).
2. Merges `*.profraw` files into `target/coverage/merged.profdata`.
3. Invokes `llvm-cov` with the right binary list to produce:
   - `target/coverage/report.lcov` (for Codecov / Coveralls)
   - `target/coverage/index.html` (human-readable)
   - Summary printed: `X of Y lines covered (Z%), X of Y fns covered`.

Refinement-type-aware coverage is a Stage-3 item: we additionally
report "how many refinement predicates were hit with both satisfying
and non-trivially-close inputs" — a higher bar than line coverage.

### 2.6 Doctest

Comments of the form:

```verum
/// Reverses a list.
///
/// @example
///   let xs = [1, 2, 3];
///   let ys = reverse(&xs);
///   assert_eq(ys, [3, 2, 1]);
```

…are extracted at `verum test` time, wrapped in `fn main() { ... }`,
compiled and run. Failure is reported as `doctest my_module::reverse`
failing. Output via `--format pretty|json|...` folds in with regular
tests.

### 2.7 CI outputs

`verum test --format junit` → standard JUnit XML at stdout or
`--output-file`, consumable by GitHub Actions, GitLab, Jenkins.

`--format tap` → TAP v13 for meta-runners and classic CI.

`--format sarif` → SARIF v2.1 for security pipelines and code
scanning dashboards; each `@property` failure becomes a `result` with
`level: "error"`, `ruleId: "verum-pbt"`, and the shrunk counterexample
in `message.text`.

## 3. Roadmap

Phased so each stage is independently useful.

### Stage 1 — **this session, committed**

- PBT runner, primitive generators (Bool, Int, Int-range, Nat, Float, Text, List, Option, Tuple)
- Integrated Hedgehog-style shrinking
- Regression DB with auto-replay
- `@property(runs=, seed=)` attribute
- Refinement-type-driven generators for `Int{lo..hi}` / `Text{len<=N}`
- Extended assertions: `assert_approx_eq`, `assert_panics`, `assert_matches`, `assert_contains`, `assert_throws`, `assert_between`, `assert_is_sorted`
- `@test_case(...)` parametrisation
- `@before_each` / `@after_each` fixtures (module-level)
- JUnit XML + TAP + SARIF output formats
- `verum coverage` integrated command
- Doctest extraction from `/// @example` blocks
- Slow-test warning (configurable threshold)
- `@snapshot` testing with `--update-snapshots`

### Stage 2 — **next sessions**

- Context-system mocking: `@test fn f() with provide { Logger = MockLogger::new() }`
- SmallCheck-style exhaustive mode for small types
- Per-test timeout override: `@timeout(ms)`
- Parallel property-testing (currently PBT is serial per-test; could fan out)
- IDE integration: LSP "Run Test" code-lens, sub-test navigation
- Watch mode: `verum test --watch`

### Stage 3 — **research territory**

- **Coverage-guided generation** (CrowBar-style, afl integration)
- **Targeted PBT** — score-function-driven simulated annealing
- **Model-based / stateful** testing over sequences of operations
- **Concurrency/race** testing with Loom-style permutation
- **Refinement-coverage metric**: percent of predicate space explored
- **Mutation testing**: `verum test --mutate` flips operators, checks the test suite actually catches the mutation

## 4. Principles we're not compromising on

1. **Zero ceremony**: `@property fn foo(x: Int) { … }` must just work.
2. **One seed → one failure**: every failure is reproducible from the
   printed seed. No hidden global state.
3. **Shrinking must converge** on minimal. Users' time is more
   expensive than CPU.
4. **No external runners required**. `cargo test` is good; our
   equivalent should not need `cargo test && external-reporter &&
   llvm-cov show && ...`.
5. **Honest status in output**. If PBT drew 100 cases and none
   exercised the `if x > 1_000_000` branch, say so.

## 5. References

- Claessen & Hughes, *QuickCheck: A Lightweight Tool for Random Testing of Haskell Programs*, ICFP 2000.
- MacIver, *Hypothesis* docs at hypothesis.readthedocs.io; see in particular the "shrinking" and "database" sections.
- Reich, *Hedgehog*, hackage.haskell.org/package/hedgehog — see the `Range`, `Gen`, and `Property` modules.
- Dolan, *Property testing with afl-fuzz* — crowbar paper; leverages coverage feedback.
- Löscher & Sagonas, *Targeted Property-Based Testing*, ISSTA 2017.
- Burckhardt et al., *Concurrency Testing with Loom*, OOPSLA 2010 — inspiration for permutation-based concurrency testing.
