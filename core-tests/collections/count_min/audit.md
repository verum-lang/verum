# `core.collections.count_min` — Audit

`core/collections/count_min.vr` — `CountMinSketch`, frequency
sketch (Cormode-Muthukrishnan 2005).  HMAC-SHA256 keyed.

## Status

**regression-only** — Construction gated on the CSPRNG defect
(same intrinsic missing-from-VBC class as Bloom / HyperLogLog /
Reservoir).  Working surface: `CountMinError` variant
construction and pattern-match — 1 unit + 1 property + 1
integration + 5 regressions (1 PASS-GUARD + 4 @ignore'd
construction-gated pins).

## Action items

* Register `core.sys.common.random_bytes` to unblock construction.
