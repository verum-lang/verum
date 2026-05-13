# `core.text.numeric.bigdecimal` — audit

> Status: **partial**. Arbitrary-precision sibling of Decimal — replaces
> the i64 coefficient with a BigInt. Inherits BigInt's defect classes;
> additionally pinned: `MAX_SCALE_BIG = 1024` (vs Decimal's 18).

## Defect classes (inherited)
Same as `core/text/numeric/bigint`.

## Action items
- 7 unit tests pinning parse surface for plain int, decimal point,
  negative, high-precision (scale > 18 — exclusive to BigDecimal),
  empty/garbage rejection, MAX_SCALE_BIG constant.
