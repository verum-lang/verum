# `core.collections.hyperloglog` — Audit

`core/collections/hyperloglog.vr` — `HyperLogLog`, cardinality
estimator (Flajolet et al. 2007).  HMAC-SHA256 keyed for adversarial-
input resistance.

## Status

**regression-only** — Construction gated on the cross-module Call
defect class shared with Bloom + CountMin + AliasSampler.  Working
surface: precision constants (`MIN_PRECISION` / `MAX_PRECISION` /
`DEFAULT_PRECISION`) and `HllError` variant construction — 4 unit
+ 4 property + 2 integration + 5 regressions (1 PASS-GUARD + 4
@ignore'd construction-gated pins).

### Re-diagnosis 2026-05-23

The blocker is NOT a missing CSPRNG intrinsic.  The cross-module
`mount core.security.util.rng.{fill_secure};` chain hits a topo-
sort gap (`core.security` depends on `core.collections`, the
back-edge is dropped, collections compiles first, fill_secure is
unresolved).  Full root-cause analysis at `core-tests/collections/
bloom/audit.md`'s "Re-diagnosis 2026-05-23" section.

Tactical workarounds (direct `core.sys.common.random_bytes` call,
qualified-name dispatch) also fail because the runtime `Call(id)`
mis-resolves through `ArchiveBodyRemap` Tier-3 identity fallback.

## Action items

* **Architectural fix — task #47**: change cross-module Call encoding
  from raw `func_id` to `StringId` (function name).  Closes this
  defect class universally across Bloom / HLL / CountMin /
  AliasSampler.
* Post-#47: property tests for `merge` precision-mismatch detection
  and add-then-estimate lower bound.
