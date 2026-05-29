# `net/http2/static_table` audit

Module: `core/net/http2/static_table.vr` (~94 LOC) — the HPACK
static table (RFC 7541 Appendix A): 61 well-known (name, value)
pairs, 1-indexed, with a bounds-checked `entry(index)` lookup.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.hpack` | combined index space 1..=61 → static. |

## 2. Crate-side hardcodes

`STATIC_TABLE_SIZE == 61` and the 61 canonical entries are RFC
7541 Appendix A verbatim. Spot-checks pin the boundaries
(indices 1, 61), the index space rules (0 and 62 → None), and
representative interior entries (`:method GET`, `:scheme https`,
`:status 200`, `accept-encoding: gzip, deflate`, `cookie`).

## 3. Language-implementation findings

The entries use `&'static Text` fields (`HpackStaticEntry`).
`entry()` returns `Maybe<HpackStaticEntry>`; field access reads
the static-Text borrow and compares against a `Text` literal.
This conformance suite verifies that comparison resolves
correctly (no static-borrow vs owned-Text drift).

## 4. Action items landed in this branch

* `unit_test.vr` — 15 tests: size + index bounds (0 / 62 / -1 /
  1 / 61); canonical (name, value) pairs at indices 1-8, 16, 32,
  61.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Full 61-entry exhaustive verification vs RFC table | this folder | 1h |
| Reverse lookup (name+value → index) helper, if added to stdlib | stdlib + tests | gated on API |
