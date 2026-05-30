# `net/http3/error` audit

Module: `core/net/http3/error.vr` — the `http3` sibling implementation's
RFC 9114 §8.1 error codes (`Http3ErrorCode`). The newer canonical impl is
`core/net/h3/` (see h3/error); this folder pins the http3 wire codes for
the duration of the parallel implementations.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http3.connection` | GOAWAY / RESET_STREAM error codes. |

## 2. Crate-side hardcodes

The 10+ H3_* codes (H3_NO_ERROR=0x100 … H3_SETTINGS_ERROR=0x109) are
RFC 9114 §8.1 IANA values, each pinned, plus the 0x1xx range invariant.

## 3. Language-implementation findings

None. `Http3ErrorCode` is a single-field record with `public const` wire
codes. Compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 10 H3 wire-code pins + 0x1xx range invariant.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Merge http3 → h3 (single canonical impl) | stdlib refactor | tracked in h3/audit §3.2 |
