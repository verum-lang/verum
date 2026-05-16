# `core.collections.alias_sampler` — Audit

`core/collections/alias_sampler.vr` — Vose's alias method for
O(1) weighted sampling.

## Status

**regression-only** — `sample()` routes through the CSPRNG.
Construction surface (`from_weights`) is pure arithmetic but
gated on the wrapper-record runtime layout class.  Working
surface: `AliasError` variant construction and pattern-match —
1 unit + 1 property + 1 integration + 4 regressions (1 PASS-GUARD
+ 3 @ignore'd construction-gated pins).

## Action items

* Same fix class as Bloom / CountMin / HyperLogLog.
* Post-fix: property tests for sample-frequency convergence over
  large samples.
