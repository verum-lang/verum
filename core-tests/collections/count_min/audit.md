# `core.collections.count_min` — Audit

`core/collections/count_min.vr` — `CountMinSketch`, frequency
sketch (Cormode-Muthukrishnan 2005).  HMAC-SHA256 keyed.

## Status

**regression-only** — Construction gated on the cross-module Call
defect class shared with Bloom + HyperLogLog + AliasSampler.
Working surface: `CountMinError` variant construction and pattern-
match — 1 unit + 1 property + 1 integration + 5 regressions (1
PASS-GUARD + 4 @ignore'd construction-gated pins).

### Re-diagnosis 2026-05-23

The blocker is NOT a missing CSPRNG intrinsic.  The cross-module
`mount core.security.util.rng.{fill_secure};` chain hits a topo-
sort gap; same root cause as Bloom — full analysis at
`core-tests/collections/bloom/audit.md`.

## Action items

* **Architectural fix — task #47**: change cross-module Call encoding
  from raw `func_id` to `StringId` (function name).  Closes this
  defect class universally across Bloom / HLL / CountMin /
  AliasSampler.
