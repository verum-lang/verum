# `core.collections.count_min` — Audit

`core/collections/count_min.vr` — `CountMinSketch`, frequency
sketch (Cormode-Muthukrishnan 2005).  HMAC-SHA256 keyed.

## Status

**partial** — 6/8 green on `--interp` (2 `@ignore`'d for
`add_increments_total` / `estimate_non_negative` populated-state
semantic, separate defect class — likely the same HMAC-SHA256
`[Byte; 64]` IndexOutOfBounds that gates Bloom §B).

**Task #47 CLOSED 2026-05-24** — cross-module Call name-encoding
via stage-3 stub pre-register + finalize-time descriptor synthesis.
Full root-cause analysis at `core-tests/collections/bloom/audit.md`.
All CountMinSketch construction tests + CountMinError variant
algebra now green.

## Action items

* (post-#47) HMAC-SHA256 `[Byte; 64]` IndexOutOfBounds fix to
  unblock the populated-state `add`/`estimate` round-trip pins.
