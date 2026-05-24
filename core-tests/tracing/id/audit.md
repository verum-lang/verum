# `tracing/id` audit

Module: `core/tracing/id.vr` (210 LOC) — W3C Trace Context
identifiers. `TraceId` is 128-bit (16-byte), `SpanId` is 64-bit
(8-byte). Both are zero-copy-encoded as fixed-size byte arrays;
the all-zero value is reserved as "invalid".

Tests cover the static surface (invalid sentinel, from_bytes,
to_hex). `from_hex` decode + generate_trace_id/generate_span_id
involve `Maybe<_>` and process-start-nanos respectively — deferred
to property_test (gated on stable `Maybe` round-trip).

## 1. Cross-stdlib usage

`TraceId` / `SpanId` consumed by:
| crate / module | what it does |
|---|---|
| `core.tracing.span` | Span struct carries TraceId + SpanId + parent SpanId. |
| `core.tracing.propagation` | W3C `traceparent` header parser / formatter — hex round-trip. |
| `core.context.standard.Tracer` | `current_trace_id()` returns Maybe<Text> (the hex form). |
| `core.tracing.exporter.{otlp,jaeger,zipkin}` | Wire-format encoding routes hex form into JSON / protobuf. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/tracing/` (when implemented) must
generate TraceId / SpanId via the canonical `generate_*` functions
or hit the same `[UInt8; N]` array layout. Drift between Rust-side
generation and the Verum-side `as_bytes()` accessor is caught at
the language level when tracing tests serialise back to hex.

## 3. Language-implementation gaps

### §3.1 `from_hex` not unit-tested

`TraceId.from_hex(&[Byte]) -> Maybe<TraceId>` decodes 32-char
lowercase hex strings. The unit tests above cover encoding
(to_hex) but not decoding. Add `from_hex` tests once
`Maybe<TraceId>` pattern matching is verified stable across
@ignore/--interp boundaries.

**Effort:** small (~30 min).

### §3.2 `generate_trace_id` / `generate_span_id` requires runtime

The generators read `process_start_nanos()` + atomic counter; pure
unit tests can't drive them deterministically without mocking the
clock + counter. Test in integration suites that provide a mock
clock context (`vcs/specs/L2-standard/tracing/`).

### §3.3 Hex encoding is lowercase per W3C — uppercase rejected

W3C Trace Context §2.2.2 mandates lowercase hex. Add a regression
test that `from_hex(uppercase_hex)` either succeeds (case-
insensitive decode, per the comment at id.vr:60) OR fails (strict
lowercase). Doc-stated behaviour is case-insensitive; pin it.

**Effort:** trivial (~15 min) once gap §3.1 closes.

### §3.4 No `Eq` / `Hash` / `Display` impls

TraceId and SpanId need Eq for `Set<TraceId>` / `Map<SpanId, _>`
lookups. Display for `f"{id}"` could return the hex form. Both
missing today.

**Effort:** small (~30 min).

## Action items landed in this branch

* `core-tests/tracing/id/unit_test.vr` — 18 unit tests covering
  invalid/from_bytes/is_invalid/to_hex/as_bytes for both TraceId
  and SpanId. Includes hex format checks (length always 32/16,
  zero/max byte rendering).
* `core-tests/tracing/id/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `from_hex` round-trip tests (encode → decode → equals) | this folder + property_test.vr | 30 min |
| Add `Eq` / `Hash` / `Display` impls to TraceId + SpanId | `core/tracing/id.vr` + tests | 30 min |
| Pin uppercase-hex behaviour (accept or reject) | `core/tracing/id.vr` + test | 15 min |
| Add generate_* tests via mock-clock context | gated on Clock context test infra | 1 day |
| Add sister tests for `core.tracing.span`, `propagation`, `sampler` | sister folders | 1 day each |
