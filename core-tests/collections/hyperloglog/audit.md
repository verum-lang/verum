# `core.collections.hyperloglog` — Audit

`core/collections/hyperloglog.vr` — `HyperLogLog`, cardinality
estimator (Flajolet et al. 2007).  HMAC-SHA256 keyed for adversarial-
input resistance.

## Status

**partial** — 14/15 green on `--interp` (1 `@ignore`'d for
`add_then_estimate` semantic, separate defect class).

**Task #47 CLOSED 2026-05-24** — cross-module Call name-encoding
via stage-3 stub pre-register + finalize-time descriptor synthesis.
Full root-cause analysis at `core-tests/collections/bloom/audit.md`.
All HLL construction tests + precision constants + HllError variant
algebra + `merge` precision-mismatch detection now green.

## Action items

* (post-#47) Property tests for `add_then_estimate` lower bound +
  union/merge associativity over compatible-precision sketches.
