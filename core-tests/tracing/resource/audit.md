# `tracing/resource` audit

Module: `core/tracing/resource.vr` (96 LOC) — OpenTelemetry
`Resource` descriptor (process/service identity attached to every
exported span / metric / log).

Tests: `unit_test.vr` (~20 unit tests covering empty/service/
from_attributes constructors + with_schema_url/with_attribute
builders + merge precedence + clone preservation).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.tracing.span.SpanProcessor` | every exported span carries the producer's Resource. |
| `core.tracing.exporter.{otlp,jaeger,zipkin}` | wire-format payload's `resource` field. |
| Application startup | `Resource.service("my-app", env!("VERSION"))` is the canonical boot pattern. |

## 2. Crate-side hardcodes

None today. `Resource` is pure Verum data layered on
`core.tracing.attribute.AttributeSet`. Future Rust-side intercepts
(zero-copy resource emission to OTLP wire format) must preserve
the attributes() / schema_url() accessor shapes.

## 3. Language-implementation gaps

### §3.1 `Resource.merge` precedence — "other wins" not pinned by spec link

The doc-comment at `resource.vr:62` cites the OTel SDK spec but
doesn't link it. The merge semantics (other's attributes override
self's, other's schema_url overrides if Some) are correct per
OTel SDK §6.3 but the unit tests above pin the surface, not the
spec. Add a property test that `merge(a, b).get(k) == b.get(k) ||
a.get(k)` for representative k.

**Effort:** small (~30 min).

### §3.2 No `Resource.Eq` impl

`Resource` has clone but not Eq. Two Resources with the same
attributes + same schema_url should compare equal. Add `Eq` impl
(or @derive once stable).

**Effort:** small (~30 min).

### §3.3 `Resource.attributes_count(&self) -> Int` shortcut missing

Today callers do `r.attributes().len()` — two-step access through
the AttributeSet. Add a direct shortcut on Resource for the hot
path. Trivial.

**Effort:** trivial (~5 min).

### §3.4 `Resource.service` shortcut doesn't accept `&Text` for both args consistently

`fn service(name: &Text, version: &Text)` — both `&Text` is
consistent, but the inner body does `name.clone()` immediately.
Could accept owned `Text` to skip the explicit clone at call
site. Or use a `&Text → Text` impl trait to allow either.
Document the choice.

**Effort:** small.

## Action items landed in this branch

* `core-tests/tracing/resource/unit_test.vr` — 20 unit tests.
* `core-tests/tracing/resource/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `property_test.vr` for merge precedence law | this folder | 30 min |
| Add `Eq` impl for Resource + sister tests | `core/tracing/resource.vr` + tests | 30 min |
| Add `attributes_count(&self) -> Int` shortcut | `core/tracing/resource.vr` + test | 10 min |
| Add sister tests for `core.tracing.{sampler,processor,exporter,span,propagation}` | sister folders | 1 day each |
