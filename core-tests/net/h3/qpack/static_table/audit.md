# `net/h3/qpack/static_table` audit

Module: `core/net/h3/qpack/static_table.vr` — RFC 9204 Appendix A
QPACK static table: 99 (name, value) entries, **0-indexed** (unlike
HPACK's 1-indexing), with `get(index)` + `find(name,value)` +
name-reference lookup.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.h3.qpack.encoder` | static-reference + name-reference encoding. |
| `core.net.h3.qpack.decoder` | static-index resolution. |

## 2. Crate-side hardcodes

`STATIC_TABLE_SIZE == 99` and the 0-indexed addressing (valid 0..=98) are
RFC 9204 §3.1 verbatim — distinct from HPACK's 61-entry 1-indexed table.
Bounds pinned (−1/99/size → None; 0/98 → Some).

## 3. Language-implementation findings

None for the covered surface. `get` is a bounds-checked `entry(index)`
lookup over `&'static` entries. Compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — STATIC_TABLE_SIZE=99 + 0-indexed bounds (−1/0/98/99/size).

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Canonical (name,value) spot-checks (RFC 9204 App. A) | this folder | 1h |
| find / name-reference lookup coverage | this folder | 1h |
