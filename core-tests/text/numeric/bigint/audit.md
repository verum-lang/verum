# `core.text.numeric.bigint` — audit

> Status: **partial**. Unit tests exercise construction, predicates, abs/neg,
> arithmetic (add/sub/mul/div_rem), and parsing. Per-test sweep deferred —
> BigInt sits on top of `List<Int>` whose iteration paths inherit the
> Iterator.next dispatch defect (text/text §C). Most arithmetic tests are
> expected to surface the same dispatch / function-id collision classes
> already pinned in `core-tests/text/text/audit.md`.

## Cross-stdlib usage
- Foundation for `core/text/numeric/bigdecimal.vr`, `rational.vr`, `modular.vr`.
- `core/cog.semver.*` uses BigInt for parts that exceed i64.

## Defect classes (inherited)
- §A — Iterator.next on `List<Int>.iter()` for digit traversal (text/text §C)
- §B — function-id collision on `add`/`sub`/`mul`/`div_rem` (text/text §D)
- §C — Int.neg / Int.* operator dispatch (Char §A class)

## Action items
- 32 unit tests + drift-pin candidates for `BIGINT_BASE = 10^9` and
  `BIGINT_DIGITS_PER_CHUNK = 9`.
- Closes when text/text §C/§D and Char §A close.
