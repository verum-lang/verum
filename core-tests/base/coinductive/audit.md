# Audit — `core/base/coinductive.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/coinductive.vr` (165 lines) |
| Tests | NEW — `unit_test.vr` (~110 LOC), `property_test.vr` (~80 LOC, is_guarded + prefix-length + round-trip) |
| Hardcodes in `crates/` | likely none — coinductive is a niche feature |

## §1  Position of this module

Coinductive types and bisimulation primitives are foundational for
working with infinite / streaming data — the canonical use case is
proving productivity (every recursive call is "guarded" by a
constructor, ensuring termination at the observable layer).

The module is small (165 LOC) and the surface is mostly ADTs +
constructors. The interesting work happens in *consumers* of these
types: theorem-proving over streams, productivity checking in
type-checked recursive definitions.

## §2  Cross-stdlib usage

A grep across `core/` finds no current consumer of these types
beyond perhaps theorem-import infrastructure in
`vcs/tools/isabelle_graph_import/`. The module exists primarily as
a foundation for future work on:
- Streaming data verification
- Coinductive proofs in the theorem importer
- Bisimulation-based equivalence checking

**Status note:** the module is *experimental* in scope. Tests pinned
here ensure the API doesn't drift, but heavy use is expected only
once theorem-import work matures.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/coinductive/`
- [x]  `unit_test.vr` — CorecursiveCall construction + is_guarded,
       check_productivity (smoke), Observation, ObservationTrace,
       trace_prefix
- [x]  `property_test.vr` — is_guarded↔depth>0 @property,
       trace_prefix length law, trace_prefix idempotence,
       Observation round-trip @property
- [x]  This audit document

## §4  Action items deferred

1. **check_productivity verdicts** — pin specific (input, expected)
   pairs once the verdict semantics are documented. Today the test
   merely calls the function and ignores the result.
2. **Bisimulation equivalence test** — when bisimulation primitives
   are wired up in consumer code, add tests verifying the
   equivalence relation laws.
3. **Theorem-import integration** — when Isabelle/Coq importers gain
   coinductive support, add cross-folder integration tests.
