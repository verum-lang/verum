# `metrics/label` audit

Module: `core/metrics/label.vr` (64 LOC) — `LabelSet` is the
cardinality dimension of Prometheus metrics. Each distinct
label-value tuple produces a distinct time series. The set is
ORDERED (insertion order) and compared value-by-value.

Tests: `unit_test.vr` (~17 unit tests covering empty/multi-value
construction, equals matrix, clone, field access),
`property_test.vr` (~10 properties — equals reflexivity/symmetry,
clone-preserves-equality, length/is_empty consistency, permutation
breaks equality).

## 1. Cross-stdlib usage

`LabelSet` is consumed by:
| crate / module | what it does |
|---|---|
| `core.metrics.registry` | `MetricFamily` keys time-series via `LabelSet`. Hot-path lookup. |
| `core.metrics.histogram` | Histogram observations carry a `LabelSet` key. |
| `core.metrics.instrument` | Counter / Gauge / Summary instruments use `LabelSet` for child series. |
| `core.metrics.prometheus` | Renders `LabelSet` to the Prometheus exposition format. |

`LabelValue = Text` alias — re-exported but semantically a Text.

## 2. Crate-side hardcodes

None today — `LabelSet` is pure Verum data. Rust-side metrics
collection (when added) would need to honour the insertion-order
semantics; this is caught by the property tests
(`property_swap_breaks_equality`).

## 3. Language-implementation gaps

### §3.1 `LabelSet.from_slice` requires `&[&Text]` 

The `from_slice` constructor takes `&[&Text]` — a slice of Text
references. Constructing this in tests is awkward; the unit tests
sidestep by using direct field-literal construction (`LabelSet {
values: [...] }`). Add an alternative constructor
`fn from_list(values: List<LabelValue>) -> LabelSet { LabelSet {
values } }` for ergonomic test construction.

**Effort:** trivial (~5 min).

### §3.2 No `LabelSet.push` for mutating builder pattern

Today the only way to extend a LabelSet after construction is to
build a fresh field-literal. Add `fn push(&mut self, value: Text)`
for builder-pattern callers (matches Prometheus client conventions).

**Effort:** trivial (~10 min) + 2 tests.

### §3.3 `LabelSet` has no `Display` / `Debug` impl

`f"{labels}"` won't compile. Adding `Display` returning the
canonical Prometheus exposition format (`{key1="val1",key2="val2"}`)
would unify rendering with `core.metrics.prometheus`. But LabelSet
stores only values (the names live separately in MetricFamily), so
the natural Display can't reproduce the key=value pairs alone —
falls back to "value1,value2,...". Document the choice; pick one.

**Effort:** small (~30 min) once decided.

### §3.4 No `LabelSet.Eq` protocol impl, only `equals` method

`LabelSet.equals(&other)` works but `labels_a == labels_b` doesn't
(no `implement Eq for LabelSet`). Adding the protocol impl
unblocks `Map<LabelSet, MetricValue>` hot-path lookups. Hash impl
follows (insertion-order-aware: order MATTERS).

**Effort:** small (~30 min) — pin both protocols.

## Action items landed in this branch

* `core-tests/metrics/label/unit_test.vr` — 17 unit tests over
  the full public API surface (new, from_slice via direct ctor,
  values, len, is_empty, equals, clone).
* `core-tests/metrics/label/property_test.vr` — 10 algebraic
  laws over equals/clone/length/permutation.
* `core-tests/metrics/label/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `LabelSet.from_list` ergonomic ctor | `core/metrics/label.vr` + tests | 10 min |
| Add `LabelSet.push` builder mutator | same | 15 min |
| Add `Eq` + `Hash` protocol impls | same | 30 min |
| Add `Display` / `Debug` impls (decide format first) | same | 30 min |
| Add sister `core-tests/metrics/{ewma,histogram,instrument,prometheus,registry,value}` suites | this folder | 1 day each |
