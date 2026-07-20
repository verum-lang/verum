# `net/http2/error` audit

Module: `core/net/http2/error.vr` (~151 LOC) — RFC 7540 §7 error
surface: `ErrorCode` (14 wire constants + new/value) and
`Http2Error` (8-variant scope-tagged ADT) with Display/Debug/Eq.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.frame` | RST_STREAM / GOAWAY carry an `ErrorCode`. |
| `core.net.http2.stream` | invalid transitions → PROTOCOL_ERROR. |
| `core.net.http2.hpack` | HPACK failures → `Http2Error.HpackError`. |

## 2. Crate-side hardcodes

All 14 ErrorCode constants are RFC 7540 §7 IANA wire values
(`NO_ERROR=0x00` … `HTTP_1_1_REQUIRED=0x0D`), each pinned by an
explicit value-equality test. Drift would break GOAWAY/RST_STREAM
interop silently.

## 3. Language-implementation findings

None. ErrorCode is a single-`UInt32`-field record; Http2Error is
a mixed record/tuple/unit-variant ADT. Both compile and dispatch
cleanly under `--interp`; the scope distinction (ConnectionError
vs StreamError) is verified disjoint.

## 4. Action items landed in this branch

* `unit_test.vr` — 33 tests: 14 ErrorCode constant pins; new/
  value/Eq; 8 Http2Error variants incl. constructor helpers,
  scope disjointness, payload preservation, Eq, Display.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ErrorCode → human reason mapping table coverage | this folder | 1h |
| Http2Error → GOAWAY frame projection round-trip | frame/ folder | gated on frame codec |
