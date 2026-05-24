# `redis/protocol` audit

Module: `core/redis/protocol.vr` (512 LOC) — RESP3 (Redis Serialization
Protocol v3) wire codec. Defines RespValue 16-variant + RespError
8-variant + encode/decode functions.

Tests: `unit_test.vr` (~33 unit tests covering all 16 RespValue
variants + all 8 RespError variants + Display rendering).

Full encode/decode round-trip tests deferred — encode is pure
fn (not requiring runtime) but exhaustive testing across the 16
variants is multi-day work.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.redis.client` | TCP frame codec for Redis connections. |
| `core.redis.commands` | builds RespValue commands. |
| `core.redis.pubsub` | Push frames for pub/sub. |
| `core.redis.script` | Lua script EVAL response decoding. |
| `core.redis.transaction` | MULTI/EXEC response Array decoding. |

## 2. Crate-side hardcodes

None today — pure Verum RESP codec. Future SIMD intercepts for
bulk-string copy (when implemented) must preserve RespValue
variant tags.

## 3. Language-implementation gaps

### §3.1 Variant naming: `Error_` and `Nil_` use trailing underscore

`RespValue.Error_` and `RespValue.Nil_` suffix-underscore to avoid
collision with `core.base.error.Error` and the conceptual `Nil` /
`Maybe.None`. Document this naming choice in the audit so future
renames preserve the trailing-underscore convention.

### §3.2 Display + Eq for RespValue not present

Only RespError has Display/Debug (already uses qualified arms per
the source). RespValue has @derive(Eq, Clone, Debug) but lacks
Display — `f"{value}"` won't compile. Add Display rendering to
the RESP3 wire format (canonical text representation for
debugging).

**Effort:** medium (~2h) — recursive structure across 16 variants.

### §3.3 No `RespValue.is_error()` / `is_null()` helpers

Common consumer pattern: `if resp.is_error() { return Err(...) }`.
Today requires explicit pattern matching. Add helpers:
* `is_simple_string` / `is_error` / `is_integer` / `is_bulk_string` /
  `is_null` (matches NullBulk OR NullArray) / `is_array` etc.

**Effort:** small (~1h).

### §3.4 `BulkString(List<Byte>)` storage cost

Holds entire payload in-memory. For pipelined GET of multi-MiB
keys, the in-memory buffer is unavoidable. Document the trade-off
+ provide streaming variant if needed.

## Action items landed in this branch

* `core-tests/redis/protocol/unit_test.vr` — 33 unit tests over
  RespValue 16-variant + RespError 8-variant + Display rendering.
* `core-tests/redis/protocol/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Display` for RespValue (canonical wire format) | `core/redis/protocol.vr` + tests | 2h |
| Add `is_*` predicate helpers for RespValue | `core/redis/protocol.vr` + tests | 1h |
| Document `Error_` / `Nil_` underscore convention | `core/redis/protocol.vr` doc | 10 min |
| Add encode/decode round-trip property tests | this folder + property_test.vr | 1 day |
| Sister tests for `core.redis.{client,commands,pubsub,stream,transaction}` | sister folders | 1 day each |
EOF
