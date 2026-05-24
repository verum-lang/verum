# `protobuf/error` audit

Module: `core/protobuf/error.vr` (86 LOC) — ProtobufError 8-variant
ADT covering protobuf wire-format decoding errors + DecodeResult<T>
alias.

Tests: `unit_test.vr` (~22 unit tests covering 8-variant construction,
disjointness, Eq matrix, Display rendering, Debug format).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.protobuf.wire` | every parse fn returns DecodeResult<T> = Result<T, ProtobufError>. |
| `core.net.http2` | gRPC over HTTP/2 uses protobuf for service framing. |
| `core.mesh.xds` | Envoy xDS payloads. |
| `core.tracing.exporter.otlp` | OTLP wire format. |

## 2. Crate-side hardcodes

None today. `core.protobuf.wire` is pure Verum.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified Display/Debug/Eq match arms

Pre-fix the match arms used bare `UnexpectedEof` / `VarintOverflow`
etc. — vulnerable to task #17/#39 first-wins collision. Qualified
to `ProtobufError.<Variant>` everywhere. Same discipline as
configuration/error, context/error.

### §3.2 No `ProtobufError.severity()` / category mapping

Distinguishing "transient" (UnexpectedEof — streamed reader should
retry) from "permanent" (InvalidUtf8 — bad payload) for retry policy
would help wire-format consumers. Similar to ConfigErrorCategory
or CacheError categorization.

**Effort:** small (~30 min) + 8 tests.

### §3.3 LengthTooLarge could carry MAX_LENGTH_DELIM const reference

The variant stores `limit: Int` — but the safety cap is a fixed
constant (`MAX_LENGTH_DELIM`). Either:
* keep the explicit limit (allows per-call tuning), OR
* drop the field + always read the global cap (simpler invariant)

Document the choice.

## Action items landed in this branch

* `core/protobuf/error.vr` — qualified all Display/Debug/Eq match arms.
* `core-tests/protobuf/error/unit_test.vr` — 22 unit tests.
* `core-tests/protobuf/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `ProtobufError.severity()` / category enum | `core/protobuf/error.vr` + 8 tests | 30 min |
| Decide LengthTooLarge.limit semantics + document | `core/protobuf/error.vr` | 15 min |
| Add property_test.vr (Display determinism, Eq reflexivity) | this folder | 30 min |
| Sister tests for `core.protobuf.wire` (LEB128 varint codec) | `core-tests/protobuf/wire/` | 2-3h |
EOF
