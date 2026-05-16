# `core.collections.hyperloglog` — Audit

`core/collections/hyperloglog.vr` — `HyperLogLog`, cardinality
estimator (Flajolet et al. 2007).  HMAC-SHA256 keyed for adversarial-
input resistance.

## Status

**regression-only** — Construction gated on the CSPRNG defect
class (Bloom / CountMin / Reservoir all share).  Working surface:
precision constants (`MIN_PRECISION` / `MAX_PRECISION` /
`DEFAULT_PRECISION`) and `HllError` variant construction — 4 unit
+ 4 property + 2 integration + 5 regressions (1 PASS-GUARD + 4
@ignore'd construction-gated pins).

## Action items

* Register `core.sys.common.random_bytes` to unblock construction.
* Post-fix: property tests for `merge` precision-mismatch detection
  and add-then-estimate lower bound.
