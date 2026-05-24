# `tracing/attribute` audit

Module: `core/tracing/attribute.vr` (146 LOC) — OpenTelemetry-
compatible attribute values + sets attached to spans, events,
links, and resources.

Tests cover:
* `AttributeValue` (8-variant): Text/Bool/Int/Float +
  TextArray/BoolArray/IntArray/FloatArray construction +
  variant dispatch via `kind()`
* `AttributeKind` (8-variant): pairwise disjoint tags
* `AttributeSet` (insertion-order key/value map):
  new/with_capacity, set/get round-trip, set-same-key-replaces
  semantics, clone preserves entries
* `ATTRIBUTE_COUNT_LIMIT` = 128 constant pin

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.tracing.span.Span` | each span carries an AttributeSet for span attributes |
| `core.tracing.span.SpanEvent` | event attributes per OpenTelemetry §events |
| `core.tracing.span.Link` | inter-span link attributes |
| `core.tracing.resource.Resource` | service-level resource attributes |
| OpenTelemetry exporters (OTLP / Jaeger / Zipkin) | wire-format encoding routes through `kind()` discriminator |

## 2. Crate-side hardcodes

None today. `AttributeValue` / `AttributeKind` are pure Verum
data; the OpenTelemetry semantic-conventions table that maps
known attribute keys (`http.method`, `db.system`, etc.) lives
in `core.tracing.span` / consumer code.

## 3. Language-implementation gaps

### §3.1 `AttributeSet.set` over-limit silently drops

When `entries.len() >= ATTRIBUTE_COUNT_LIMIT` (128), `.set(...)`
returns without inserting. This matches OpenTelemetry's
"dropped attribute" semantics but offers no observability —
callers can't tell that a drop occurred. Add a return value:
`fn set(&mut self, ...) -> Bool` (true = accepted, false = dropped).

**Effort:** small (~30 min) + 2 boundary tests.

### §3.2 `AttributeSet.get` is O(n) linear scan

Linear scan is acceptable up to ~32 entries (OpenTelemetry
default limit). For 128-entry sets (our hard cap) lookups
degrade. Consider:
* `Map<Text, AttributeValue>` storage for larger sets, OR
* Insertion-order linked list + hash map index (best of both)

Today's choice matches the spec's "ordered linear scan up to
the limit" pattern; pin the trade-off.

### §3.3 No `AttributeValue.Eq` round-trip tests

The unit tests cover `is`-based variant matching but not
structural equality (`a == b`). Adding Eq round-trip tests
requires `Eq` impl which the doc-stated derivation should
provide; verify with `@derive(Eq, Clone, Debug)` once stable.

### §3.4 `AttributeValue.Text(Text)` shadows the host `Text` type

The variant name `Text` collides with the host type `Text` —
bare `Text(...)` in pattern matching could mis-resolve. The
module uses qualified `AttributeValue.Text(_)` in match arms
(verified at `attribute.vr:39-44`), but consumers writing
their own match arms must use the qualified form. Pin this
hazard in audit.

## Action items landed in this branch

* `core-tests/tracing/attribute/unit_test.vr` — 25 unit tests
  covering AttributeValue 8-variant + kind() dispatch + Kind
  variants + AttributeSet new/get/set/len/clone + LIMIT pin.
* `core-tests/tracing/attribute/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Make `AttributeSet.set` return Bool drop-indicator | `core/tracing/attribute.vr` + tests | 30 min |
| Add `Eq` round-trip tests for AttributeValue variants | this folder + property_test.vr | 30 min |
| Add property_test.vr (kind() determinism, clone preserves kind) | this folder | 30 min |
| Document `AttributeValue.Text` host-type collision in module doc | `attribute.vr` doc comment | 10 min |
| Consider `Map<Text, AttributeValue>` storage for >32-entry sets | `attribute.vr` | 1 day |
