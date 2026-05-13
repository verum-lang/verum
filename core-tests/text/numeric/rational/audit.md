# `core.text.numeric.rational` — audit

> Status: **partial**. Sits on top of BigInt; inherits the
> Iterator.next + function-id collision defects from text/text §C/§D.

## Defect classes (inherited)
Same as `core/text/numeric/bigint`. Closes when those close.

## Action items
- 6 unit tests pinning the parser surface (simple int, simple fraction,
  negative fraction, /0 rejected, empty rejected).
- Once arithmetic round-trips work, expand with property tests:
  exact-arithmetic invariants (1/3 + 1/3 + 1/3 = 1, a/b + c/d = (ad+bc)/bd
  with canonical reduction).
