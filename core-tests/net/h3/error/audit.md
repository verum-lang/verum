# `net/h3/error` audit

Module: `core/net/h3/error.vr` — RFC 9114 §8.1 + RFC 9204 §6 error
codes: `H3ErrorCode` (17 H3 + 3 QPACK wire constants + new/value) and
the `H3Error` ADT (connection/stream-scoped + frame/stream decode +
streaming-body errors).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.h3.connection` | GOAWAY / RESET_STREAM carry an H3ErrorCode. |
| `core.net.h3.frame` | frame-decode failures → H3Error.FrameDecode. |
| `core.net.h3.qpack` | QPACK_* codes on the encoder/decoder streams. |

## 2. Crate-side hardcodes

All 17 H3 codes (H3_NO_ERROR=0x0100 … H3_VERSION_FALLBACK=0x0110) and the
3 QPACK codes (0x0200-0x0202) are RFC 9114 §8.1 / RFC 9204 §6 IANA wire
values, each pinned. The H3 (0x01xx) vs QPACK (0x02xx) range separation is
pinned by `test_h3err_h3_range_below_qpack_range`.

## 3. Language-implementation findings

None. H3ErrorCode is a single-`UInt64`-field record; H3Error is a mixed
record/tuple/unit-variant ADT. Compiles and dispatches cleanly under
`--interp`. (The h3 umbrella suite covers H3FrameError + UniStreamType;
this folder pins the error-code surface specifically.)

## 4. Action items landed in this branch

* `unit_test.vr` — 17 H3 + 3 QPACK code pins; new/value; H3 vs QPACK
  range disjointness; H3Error variants (ConnectionError/MissingSettings)
  + scope disjointness + Display.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| H3Error full ~12-variant Eq matrix | this folder | 1h |
| H3ErrorCode → GOAWAY frame projection round-trip | h3/frame | gated on transport harness |
