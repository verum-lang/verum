# `core-tests/` — Battle-grade tests for the Verum standard library

> Pure Verum tests. No frontmatter, no `vtest`-style directives. Run with
> `verum test --interp` (Tier 0 interpreter) and `verum test --aot`
> (Tier 2 AOT, default). The same source must pass both.

## What lives here

`core-tests/` is the home of the **stdlib conformance suite** — the thing
the Verum project itself uses to keep `core/` honest. It is *not*
project-application tests (those live in `tests/`); it is *not* the legacy
specification suite (that lives in `vcs/specs/` and runs through `vtest`
with comment frontmatter).

Every `core/<x>/<y>.vr` source file gets a matching folder
`core-tests/<x>/<y>/` with a small set of test files inside it. Discovery
is automatic: `verum test` walks `tests/` and `core-tests/` together
(`crates/verum_cli/src/commands/test.rs::test_source_dirs`).

## Why this matters

These tests aren't decoration — they're the lever for evolving Verum.

* **Defects in the standard library** surface here first. Each module
  gets its own `audit.md` with severity-ranked findings.
* **Defects in the language** also surface here, because stdlib code
  exercises every corner of the type system, codegen, and runtime. When
  a property test for `Maybe` triggers a method-resolution bug, that's a
  language-level issue captured by a stdlib-level test.
* **Cross-tier divergence** — anything that interpreter and AOT disagree
  on is a kernel-soundness incident. The fact that we run *both* makes
  that drift detectable.

So when you write tests here, you're not "checking that the code works."
You're constructing the **contract** the standard library promises and
the **probe** that finds the gap between the contract and the
implementation. Treat unexpected failures as findings, not annoyances.

## Layout — one folder per stdlib file

```
core-tests/<x>/<y>/
├── unit_test.vr          # API surface; one @test per public method
├── property_test.vr      # algebraic / mathematical laws (exhaustive
│                         #   when the domain is finite, sampled otherwise)
├── integration_test.vr   # cross-type scenarios with the rest of stdlib
├── regression_test.vr    # one @test per past defect; never deleted
└── audit.md              # findings + drift surfaces + action items
```

Mirror is strict. `core/base/ordering.vr` ↔ `core-tests/base/ordering/`.
The mirror is the discovery contract.

The `ordering/` folder is the **canonical worked example** — copy its
shape for every new module.

## File conventions — write pure Verum, no metadata

Tests are plain `.vr` files. No frontmatter. The first line should be
either a `mount` import or a one-line banner; nothing else.

```verum
mount core.base.ordering.{Ordering, Less, Equal, Greater};

@test
fn property_reverse_involution_exhaustive() {
    let xs: List<Ordering> = [Less, Equal, Greater];
    for x in xs.iter() {
        assert_eq((*x).reverse().reverse(), *x);
    }
}

fn main() {
    // verum test synthesises a per-test main; this stays for the
    // case where the file is also runnable directly. Optional.
}
```

What is **forbidden**:

* `// @test:`, `// @level:`, `// @tier:`, `// @tags:`, `// @description:`
  and any other vtest-style frontmatter — these belong in `vcs/specs/`.
* `module specs.stdlib.…;` declarations — vtest legacy.
* `#[test]` — Verum uses `@test` (the AST attribute), not Rust syntax.
* `assert!`, `panic!`, `format!`, `println!` — these `!` macro forms do
  not exist in Verum; use `assert`, `panic`, `f"..."`, `print`.

## How to run

```bash
# Default: AOT (each test file → native binary → exit code 0 == pass).
verum test

# Interpreter (Tier 0, in-process, fastest iteration):
verum test --interp

# Filter by substring in test name:
verum test --filter property_reverse

# Run a specific module's tests:
verum test --filter ordering::

# List discovered tests without running:
verum test --list

# Don't capture stdout (useful for debugging):
verum test --interp --nocapture
```

The contract: every `@test` in `core-tests/` must pass under **both**
`--interp` and `--aot`. Disagreement is a tiered-execution kernel
incident — file it.

## How to write each file

### `unit_test.vr` — API surface

* One `@test` per public method.
* One axis of behaviour per `@test`. Don't smush "method works on Some"
  and "method works on None" into one test — separate them so the
  failure message names the case directly.
* Group by method, with `// === Section N: <method> ===` banners.

If a future reader can't reconstruct the public API by reading just the
section banners, the file is too thin.

### `property_test.vr` — laws, not examples

Verum's protocol design makes finite domains common (`Ordering` is
3-valued; small enums; bit-flags). For these, **exhaust** the Cartesian
product:

```verum
fn domain() -> List<Ordering> { [Less, Equal, Greater] }

@test
fn law_then_associative_exhaustive() {
    for a in domain().iter() {
        for b in domain().iter() {
            for c in domain().iter() {
                assert_eq((*a).then(*b).then(*c),
                          (*a).then((*b).then(*c)));
            }
        }
    }
}
```

For larger domains, switch to representative-sample sweeps (boundaries,
mid-range values, signed extremes). Document the sampling rationale
inline. When Verum gains a real quickcheck, swap in.

Standard laws to consider per protocol:

| Protocol | Laws |
|---|---|
| `Eq` | reflexive, symmetric, transitive; `ne` is exact negation of `eq` |
| `Ord` | total order; `cmp` anti-symmetric; `lt/le/gt/ge` cohere with `cmp`; `min/max/clamp` invariants |
| `Hash` | `a == b ⇒ hash(a) == hash(b)`; deterministic across calls |
| `Clone` | `a.clone() == a`; clone is independent of source |
| `Default` | deterministic; matches per-type expected default |
| `From/Into` | reflexive `From<T> for T`; `into ⇔ U.from` |
| `Add/Sub/Mul` | commutativity (where applicable), associativity, identities, distributivity |
| `BitAnd/BitOr/BitXor/Not` | idempotence, self-inverse, De Morgan |
| `Try/FromResidual` | `branch ∘ from_output = Continue`; residual round-trip |
| Module-specific | round-trip invariants, monotonicity, absorption, etc. |

### `integration_test.vr` — meeting other stdlib types

Touch real stdlib types — `List.sort`, `Map.insert/get`, `Set.contains`,
`Maybe.cmp`, formatter output. **Don't mock the stdlib.** If you can
write the test against a synthetic mock, fold it back into `unit_test`.

### `regression_test.vr` — pin every bug

Every defect we have ever seen gets a one-test-per-bug entry. Keep the
test forever — even if the language fixes the underlying issue. The
test then becomes a guardrail against re-regression.

Each test starts with a comment naming the defect:

```verum
// REGRESSION: pre-fix, `for ord in xs.iter() { ord.reverse() }` failed
// with "method not found" because the iterator-item type was inferred
// too late for method dispatch. Workaround: explicit `(*ord).reverse()`.
// Tracked in audit.md §3.1.
@test
fn regression_iterator_item_method_resolution() { … }
```

### `audit.md` — findings, not decoration

Every module's `audit.md` is structured around three audits:

1. **Cross-stdlib usage** — where else in `core/` is this type touched?
   Are consumers correct, idiomatic, redundant?
2. **Crate-side hardcodes** — search `crates/` for sites that hardcode
   names, tags, or method signatures of this type. List every drift
   surface with `path:line`.
3. **Language-implementation gaps** — defects in type-inference,
   codegen, or runtime that the tests in this folder pin.

End with:

* **Action items landed in this branch** — what was actually fixed.
* **Action items deferred** — ranked, with scope estimates.

Use the `core-tests/base/ordering/audit.md` and
`core-tests/base/protocols/audit.md` files as templates.

## Concurrent agent workflow

This suite is meant to be filled in **parallel** by independent agents,
each owning one stdlib module. Coordination protocol:

### Claim → work → validate → land

1. **Claim** — list pending tasks (`TaskList` if you're using the task
   harness, or `git status core-tests/` to see what's missing). Pick a
   module with no `core-tests/<...>/<module>/` folder and no in-progress
   task. Update task status to `in_progress` and set yourself as owner.

2. **Audit first, code second** — for each module:
   * Read the source: `core/<...>/<module>.vr`.
   * Read existing tests in `vcs/specs/` if any (greppable as
     `<module>_test.vr`); these may need migration into `core-tests/`
     and may already cover the API surface.
   * Spawn two parallel audit agents:
     * **Cross-stdlib** — where is this module used? Are there
       redundant or anti-pattern call sites? (Subagent: `Explore`.)
     * **Crate-side hardcodes** — what does Rust code in `crates/`
       hardcode about this type? Drift surfaces? (Subagent: `Explore`.)
   * Distill findings into a draft `audit.md`.

3. **Migrate first if existing tests are present**:
   * `git mv vcs/specs/.../<module>_test.vr core-tests/<...>/<module>/unit_test.vr`
   * Strip frontmatter (`// @test:`, `// @level:`, etc.) — pure Verum
     only. The `// =================…=` banners and ordinary comments
     stay.

4. **Write `property_test.vr`** with module-relevant algebraic laws.
   Exhaustive when feasible; sampled otherwise; document the choice.

5. **Write `integration_test.vr`** exercising cross-type interactions.

6. **Apply easy fixes from the audit** in the same branch — typically:
   * Drift-pinning unit tests in the relevant Rust crate (mirror the
     `ORDERING_VARIANT_LAYOUT` / `primitive_protocol_matrix_pinned`
     patterns in `crates/verum_common/src/well_known_types.rs`).
   * Idiomatic refactors removing redundant `match { Equal => true,
     _ => false }` helpers, raw-`Int` `cmp` returns, manual lex chains
     where `.then()` would do, etc.
   * **Defer** cross-cutting changes (multi-module API renames,
     codegen rewrites) — log them as `Action items deferred` in
     `audit.md`.

7. **Validate locally** with **both** tiers:
   ```bash
   verum test --interp --filter <module>::
   verum test --aot    --filter <module>::
   ```
   Both must be green. If they disagree, open the failure analysis,
   pin the disagreement in `regression_test.vr`, then escalate the
   underlying kernel/codegen issue.

8. **Land** — update the task to `completed`, write a tight commit
   message that names: (a) the module under test, (b) the audit
   findings landed, (c) the deferred items.

### Avoid stepping on each other

* **One module per agent at a time.** The folder boundary is the
  isolation unit.
* **Don't edit `core/<other>/<other>.vr`** without owning that module's
  task — flag the cross-module finding in your `audit.md` instead.
* **Drift-pinning tests in `crates/verum_common`** are shared
  infrastructure; coordinate via the well-known-types module's
  internal CLAUDE.md if you need to add a new pinning macro.
* **Don't push to `main` mid-cycle** — squash-merge via PR or land in
  topic branches per module.

### Typical timeline per module

| Module size | Read | Migrate | Audit | property | integration | Fixes | **Total** |
|---|---|---|---|---|---|---|---|
| Small (< 300 LOC src) | 5 min | 2 min | 15 min | 30 min | 20 min | 10 min | ~80 min |
| Medium (300–1500) | 15 min | 5 min | 30 min | 60 min | 40 min | 30 min | ~3 h |
| Large (> 1500, e.g. iterator) | 45 min | 20 min | 60 min | 2 h | 90 min | 1 h | ~6 h |

These are guidelines, not contracts. Iterators and primitives blow past
the upper end; nanoid/snowflake fall well below the lower.

## When tests fail

Failures fall into a small number of buckets. Use this triage:

| Symptom | First-pass diagnosis |
|---|---|
| `--interp` passes, `--aot` fails | Codegen miscompile or LLVM lowering bug. Pin in `regression_test.vr`. |
| `--aot` passes, `--interp` fails | VBC interpreter dispatch / intrinsic missing. Pin and escalate. |
| Both fail with `method not found` | Stdlib API mismatch: missing impl or wrong receiver type. Source-level fix. |
| Both fail with type-inference error | Audit `regression_test.vr` for known type-inference issues; if new, add. |
| Hangs / times out | `verum test --interp --nocapture` to see where. Possibly an iterator producing infinite values. |
| Spurious panic in `assert_eq` | Read the panic carefully; the laws may have caught a real bug — don't weaken the test. |

When unsure whether the bug is in your test or in the stdlib, write the
**simplest reproducer** in `regression_test.vr` and let the audit
process route it.

## What we explicitly do not test here

* **Compiler error messages** — those live in `vcs/specs/` with vtest
  frontmatter (`@expected-error: E400`).
* **Whole-program performance benchmarks** — those live in `vcs/benchmarks/`
  via `vbench`.
* **Internal compiler crate behavior** — that lives in
  `crates/<crate>/tests/` as Rust unit tests.
* **VBC bytecode encoding** — Rust-level concern; tested in
  `verum_vbc/src/` via `cargo test`.

## Status & inventory

A live inventory of which modules have folders here, the LOC of each
test file, and the open audit deferrals lives in
`core-tests/INVENTORY.md` (kept up-to-date by whichever agent finishes a
module — append a single row, don't restructure).

If `INVENTORY.md` does not exist yet, create it on your first commit.
Single-line rows: `<module> | <unit_loc> | <property_loc> | <integration_loc> | <audit deferrals>`.

---

That's the whole contract. Read `core-tests/base/ordering/` once,
internalise the layout, claim a task, and ship.
