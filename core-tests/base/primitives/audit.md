# Audit — `core/base/primitives.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/primitives.vr` (6665 lines — LARGEST in `core/base/`) |
| Tests | `core-tests/base/primitives/` — `int_test.vr` (875), `float_test.vr` (997), `bool_byte_char_test.vr` (839), `comparison_test.vr` (1057), `property_test.vr` (NEW, ~210 LOC, cross-primitive laws) |
| Hardcodes in `crates/` | massive — primitives are language axioms; their type IDs and method dispatch hit every codegen path |

## §1  primitive_implements_protocol matrix

The single most important drift surface for primitives lives in
`crates/verum_common/src/well_known_types.rs:718-778` — the
`primitive_implements_protocol(type, protocol)` function. It hardcodes
which primitives satisfy which protocols.

This matrix was pinned by a unit test landed earlier in this branch
(`primitive_protocol_matrix_pinned`) — see
`core-tests/base/protocols/audit.md §2.1`.

Any new primitive added to the language MUST update this matrix;
the test will fail otherwise.

## §2  Per-primitive method coverage

Primitives have ~6665 LOC of stdlib methods because every numeric
type carries arithmetic, comparison, conversion, formatting, and
specialised helpers (saturating, wrapping, checked, leading_zeros,
trailing_zeros, count_ones, …).

The four migrated test files cover:
- `int_test.vr` — Int arithmetic, comparison, conversion
- `float_test.vr` — Float IEEE 754 conformance, NaN, Inf, denormals
- `bool_byte_char_test.vr` — Bool boolean-algebra, Byte arithmetic-mod-256, Char Unicode
- `comparison_test.vr` — comparison-operator type inference (ensures `<` returns Bool, not Ordering)

`property_test.vr` (new) adds cross-primitive laws — Int commutativity,
Float NaN semantics, Bool De Morgan, Char round-trip, Byte wrap, Int
parse round-trip.

## §3  Ratio coverage

Source 6665 LOC vs tests 3768+210 = 3978 LOC ≈ 0.6× ratio. The norm
across other modules is 1.5–2× ratio. The shortfall is in:

- UInt8/16/32/64 — partially covered via Byte tests but not exhaustive
- ISize/USize — minimal coverage
- Bit-manipulation methods (count_ones, leading_zeros, etc.) — sparse
- Saturating / Wrapping / Checked arithmetic — sparse
- Float-specific edge cases (subnormals, underflow) — sparse

**Action item (deferred):** add per-method-family test files for
each gap. Not critical but would close the coverage shortfall.

## §4  Action items landed in this branch

- [x]  Migrate four test files from `vcs/specs/core/core/`
- [x]  Strip vtest frontmatter
- [x]  Add `property_test.vr` covering cross-primitive laws (Int
       arithmetic identity, Float NaN, Bool boolean algebra, Char
       round-trip, Byte mod-256, Int to-string round-trip),
       plus @property samples and @test_case truth tables
- [x]  Add this audit document

## §5  Action items deferred (not landed)

1. **UInt / ISize / USize coverage** — partial today; need dedicated
   per-width test files.
2. **Bit-manipulation laws** — count_ones / count_zeros / leading_zeros /
   trailing_zeros / rotate_left / rotate_right; algebraic invariants
   (`x.count_ones() + x.count_zeros() = bit_width`, etc.).
3. **Saturating/Wrapping/Checked arithmetic** — pin the contract: for
   any overflow, saturating produces MAX/MIN, wrapping produces the
   modular result, checked returns None.
4. **Float edge cases** — denormals, ±Inf, subnormal-to-zero
   transitions, FMA fused-multiply-add accuracy.
5. **Reference: cross-tier divergence** — primitive arithmetic is the
   most likely site for interp-vs-AOT mismatch (different IEEE rounding
   modes, different overflow semantics). Once differential testing
   lands (`CAPABILITY_GAPS.md §3.1`), primitives is the first place
   to look for hidden bugs.
