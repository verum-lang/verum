# Audit — `core/base/ordering.vr`

> Implementation audit consolidating cross-stdlib usage (§1), language-impl
> hardcodes in the Rust crates (§2), and language-level type-inference defects
> exposed by `regression_test.vr` (§3). Each finding ends with a suggested
> remediation; severity-marked items in §2 are blockers for variant reordering.

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/ordering.vr` (196 lines) |
| Tests | `core-tests/base/ordering/` — `unit_test.vr` (1014 LOC, migrated from `vcs/specs/core/core/ordering_test.vr`), `property_test.vr` (exhaustive 3¹/3²/3³ over algebraic laws), `integration_test.vr` (cross-type), `regression_test.vr` (pinned defects) |
| Importers in stdlib | 14 files |
| `Ord`/`PartialOrd` `cmp` impls examined | 16 |
| Hardcodes in `crates/` | 9 sites — 2 critical (variant tags), 7 benign |

## §1  Stdlib usage findings

### 1.1  CRITICAL — `cmp`-shaped functions returning raw `Int` instead of `Ordering`

These violate the Ord-protocol type discipline. They typecheck because they're free functions, not protocol method bodies, but they prevent stdlib consumers from chaining via `.then()`/`.reverse()`/`.is_le()`.

| File | Line | Defect |
|---|---|---|
| `core/base/semver.vr` | 327 | `cmp(a, b) -> Int` returning -1/0/1 |
| `core/database/sqlite/native/snapshot_api/handle.vr` | 40 | same |
| `core/database/sqlite/native/snapshot_isolation_api/handle.vr` | n/a | same (mirror of above) |
| `core/math/big_uint.vr` | 202 | `cmp(&self, other) -> Int` returning -1/0/1 |

**Fix:** change the return type to `Ordering` and rewrite call sites
(`Ordering.from_int(...)` is available as the lossy bridge if needed).

### 1.2  Redundant `match { Equal => true, _ => false }` patterns

Five files reimplement what `is_equal()`/`is_less()`/`is_greater()` already provide.
Each of these is a 3-line dead helper:

| File | Lines | Helper |
|---|---|---|
| `core/money/money.vr` | 354–356 | `matches_eq(o)` etc. |
| `core/text/numeric/rational.vr` | 458–460 | three-way clones |
| `core/text/numeric/bigdecimal.vr` | 555–557 | three-way clones |
| `core/text/numeric/bigint.vr` | 1208–1216 | three-way clones |
| `core/text/numeric/decimal.vr` | (mirror) | three-way clones |

**Fix:** delete the helpers; replace call sites with the protocol methods.
Reduces stdlib surface area by ~50 lines and eliminates a maintenance hazard
(if anyone changes `Ordering` variants, these all silently break).

### 1.3  Manual if-else duplicating `Ordering.from_int`

| File | Line | Pattern |
|---|---|---|
| `core/math/epistemic.vr` | 168–174 | `if l < r { Less } else if l > r { Greater } else { Equal }` |

**Fix:** `Ordering.from_int((l as Int) - (r as Int))` if the rank type is
integer-coercible — single line, intent-revealing.

### 1.4  Manual lexicographic chain instead of `.then()`

| File | Line | Defect |
|---|---|---|
| `core/collections/list.vr` | 2767–2785 | `cmp_lexicographic` uses `match c { Equal => {} _ => return c }` |
| `core/meta/span.vr` | 268–271 | `match start { Equal => end_cmp, ord => ord }` |

**Fix:** `start_cmp.then(end_cmp)` (or `then_with` if `end_cmp` is expensive).
Idiomatic and shorter; promotes the API the language wants users to reach for.

### 1.5  Duplicate ordering enum (`KeyOrdering`)

| File | Lines |
|---|---|
| `core/database/sqlite/native/l3_btree/comparator.vr` | KoLess/KoEqual/KoGreater |
| `core/database/sqlite/native/l3_btree/record_compare.vr` | imports KeyOrdering |

**Diagnosis:** legitimate domain wrapper *or* unconverted legacy depending on
intent. If Sqlite-internal records semantically differ from generic `Ordering`
(e.g. they represent collation results pre-Unicode), keep the wrapper but add
`From<KeyOrdering> for Ordering` and document the boundary. If they're just
duplication, remove and use `Ordering`.

**Suggested action:** treat as low-priority cleanup; investigate intent before
removing.

## §2  Language-impl hardcodes (`crates/`)

### 2.1  CRITICAL — Variant-tag drift surface

The variant ordering `Less=0, Equal=1, Greater=2` is hardcoded in **two
independent Rust sites** that must stay synchronised with `core/base/ordering.vr`:

```text
crates/verum_vbc/src/codegen/mod.rs:4939-4941
    ("Ordering", "Less",    0, 0, vec![]),
    ("Ordering", "Equal",   1, 0, vec![]),
    ("Ordering", "Greater", 2, 0, vec![]),

crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs:2265-2282
    let tag = match ord {
        std::cmp::Ordering::Less    => 0u32,
        std::cmp::Ordering::Equal   => 1u32,
        std::cmp::Ordering::Greater => 2u32,
    };
```

**Failure mode:** if anybody edits the source-of-truth `.vr` to declare e.g.
`Equal | Less | Greater`, the codegen emits tag `0` for `Equal`, but the runtime
constructor still maps `std::cmp::Ordering::Less` to tag `0`. Result: silent
data corruption — every `cmp(a, b)` would return the wrong variant, every sort
would produce garbage, with no error message.

**Architectural fix (recommended):**
1. **Add a load-time invariant check.** When the stdlib is loaded, look up
   `Ordering` in the metadata, walk its variants, assert that `variants[0].name
   == "Less" && variants[0].tag == 0` etc. Fail loudly on drift. Tracked as a
   landed change — see `crates/verum_vbc/src/interpreter/validation/ordering_layout.rs`
   in this branch.
2. **Centralise the canonical layout** in `verum_common::well_known_types` as
   a single `const ORDERING_LAYOUT: &[(&str, u32)] = &[("Less", 0), ("Equal", 1),
   ("Greater", 2)]` referenced from both codegen and runtime. Drift → compile error.

### 2.2  Benign hardcodes (keep, but document)

| Site | Why benign |
|---|---|
| `verum_common/src/well_known_types.rs:62` | Just the type-name registration; no semantics |
| `verum_codegen/src/llvm/instruction.rs` (×2) | `"Ordering"` in receiver-type allowlist for cmp dispatch — no layout dependency |
| `verum_compiler/src/phases/dependency_analysis.rs` | Declares `Ordering` as no-allocation builtin (correct: all variants are unit) |
| `verum_types/src/infer_path_resolution.rs` | Disambiguation priority — name-only |
| `verum_interactive/src/discovery/index.rs:271` | UI metadata only |

These are legitimate "this compiler knows about this type" hooks. Keep them.
The danger is *exclusively* in §2.1.

### 2.3  `Ordering.from_int` is correctly *not* hardcoded

The body lives in `core/base/ordering.vr:113-117`. Compile path goes through
the normal stdlib loader. No drift risk.

## §3  Language-level defects exposed by `regression_test.vr`

### 3.1  Iterator-item method dispatch

`regression_test.vr::test_iterator_deref_reverse` pins this:

```verum
for ord in orderings.iter() {
    ord.reverse();           // FAILS: method not resolved
    (*ord).reverse();        // works (manual deref)
}
```

The iterator-item type is *inferred* but not *resolved* early enough for
method-lookup. This is a defect in `crates/verum_types/src/infer.rs` (likely in
the `for`-loop iteration variable's lazy unification with the iterator's
`Item` projection).

**Action:** filed as a separate language-level issue — out of scope for this
audit. Test pinned. Once fixed at language level, the workaround `(*ord).method()`
in `unit_test.vr` Section 23 / 24 / 25 can be removed.

## §4  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/ordering_test.vr` → `core-tests/base/ordering/unit_test.vr`
- [x]  Migrate `vcs/specs/core/core/ordering_minimal_test.vr` → `core-tests/base/ordering/regression_test.vr`
- [x]  Add `property_test.vr` (algebraic-law exhaustive verification)
- [x]  Add `integration_test.vr` (cross-type — primitives, Maybe, sort, lex chain, three-way partition, binary search)
- [x]  Add load-time invariant check (`crates/verum_vbc/src/interpreter/validation/ordering_layout.rs`) closing §2.1 drift surface
- [x]  Add this audit document

## §5  Action items deferred (not landed in this branch)

These are real, ranked, but require coordination with downstream call sites.

1. Convert §1.1 raw-`Int` `cmp` functions to return `Ordering`. **Scope:** 4
   call sites + their consumers. Cross-cutting.
2. Delete §1.2 redundant `match { Equal => true, _ => false }` helpers.
   **Scope:** 5 files, ~20 call sites.
3. Replace §1.3 manual if-else with `Ordering.from_int`.
   **Scope:** 1 file.
4. Replace §1.4 manual `match c { Equal => …, _ => return c }` with `.then()`.
   **Scope:** 2 files.
5. Investigate §1.5 `KeyOrdering` — keep with explicit `From` impl or remove.
   **Scope:** 2 files, requires intent check with sqlite subsystem owner.

## §6  Test-infrastructure recommendations (pinned for follow-up)

From the broader `vtest` audit (companion task), these defects affect *how*
the tests in this directory run rather than the tests themselves. Pinned for
attention in the next iteration:

- `@test` lives in comments, not in the AST attribute system → no type-checker
  enforcement of test signature shape.
- No real test isolation — global `FileId` counter and stdlib cache leak
  between tests.
- "Differential testing" across tiers is aspirational; the executor doesn't
  actually compare cross-tier outputs.
- No property-test framework — currently exhaustive sweeps (like
  `property_test.vr`) are the only available substitute.
- No structured error-code registry; `@expected-error: E999` typo silently
  passes.

These are tracked separately in `core-tests/CLAUDE.md` (next).
